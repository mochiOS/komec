use kome_native_rt::number::Number;
use kome_native_rt::string::KomeString;
use kome_native_rt::{
    RuntimeError, Value, builtin_registry, clear_thread_registry, set_thread_registry,
};
use std::os::fd::AsRawFd;
use std::sync::{Arc, Mutex};

#[test]
fn sleeps_without_blocking_other_kome_tasks() {
    let values = run_with_registry(
        r#"
@native("test.capture")
fn report(value: String)
@native("io.sleep")
fn sleep(milliseconds: Number) -> Null
fn delayed() -> String {
    sleep(5)
    return "after"
}
fn immediate() -> String { return "before" }
fn main() {
    let delayed_task = task delayed()
    let immediate_task = task immediate()
    report(wait immediate_task)
    report(wait delayed_task)
}
"#,
        None,
    );
    assert_eq!(
        values,
        vec![
            Value::String(KomeString::new("before")),
            Value::String(KomeString::new("after")),
        ]
    );
}

#[test]
fn socket_method_suspends_until_data_is_readable() {
    let sockets = std::os::unix::net::UnixStream::pair().unwrap();
    let values = run_with_registry(
        r#"
@native("test.capture")
fn report(value: String)
@native("test.reader_fd")
fn readerFd() -> Number
@native("test.writer_fd")
fn writerFd() -> Number
@native("io.socket_read")
fn socketRead(fd: Number, maximum: Number) -> String
@native("io.socket_write")
fn socketWrite(fd: Number, value: String) -> Number
struct Socket { fd: Number }
for Socket {
    fn read(self) -> String { return socketRead(self.fd, 64) }
    fn write(self, value: String) -> Number { return socketWrite(self.fd, value) }
}
fn main() {
    let reader = Socket { fd: readerFd() }
    let writer = Socket { fd: writerFd() }
    let response: Task<String> = task reader.read()
    let sent: Task<Number> = task writer.write("hello")
    report(wait response)
    let bytes = wait sent
}
"#,
        Some(sockets),
    );
    assert_eq!(values, vec![Value::String(KomeString::new("hello"))]);
}

#[test]
fn cancelling_a_suspended_socket_task_unwinds_through_task_cleanup() {
    let sockets = std::os::unix::net::UnixStream::pair().unwrap();
    let values = run_with_registry(
        r#"
@native("test.capture")
fn report(value: Number)
@native("test.reader_fd")
fn readerFd() -> Number
@native("io.sleep")
fn sleep(milliseconds: Number) -> Null
@native("io.socket_read")
fn socketRead(fd: Number, maximum: Number) -> String
struct Socket { fd: Number }
for Socket {
    fn read(self) -> String { return socketRead(self.fd, 64) }
}

fn cancelAfter(value: Task<String>) -> Number {
    sleep(1)
    cancel value
    return 1
}
fn main() {
    let reader = Socket { fd: readerFd() }
    let response: Task<String> = task reader.read()
    let canceller: Task<Number> = task cancelAfter(response)
    report(wait canceller)
}
"#,
        Some(sockets),
    );
    assert_eq!(values, vec![Value::Number(Number::from_i64(1))]);
}

#[test]
fn runtime_socket_connects_reads_writes_and_closes() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let (mut connection, _) = listener.accept().unwrap();
        let mut request = [0_u8; 4];
        connection.read_exact(&mut request).unwrap();
        assert_eq!(&request, b"ping");
        connection.write_all(b"pong").unwrap();
    });
    let source = format!(
        r#"
@runtime("socket")
struct Socket
@native("io.socket_connect")
fn socketConnect(host: String, port: Number) -> Socket
@native("io.socket_read")
fn socketRead(socket: Socket, maximum: Number) -> String
@native("io.socket_write")
fn socketWrite(socket: Socket, value: String) -> Number
@native("io.socket_close")
fn socketClose(socket: Socket) -> Null
for Socket {{
    fn connect(host: String, port: Number) -> Socket {{
        return socketConnect(host, port)
    }}
    fn read(self, maximum: Number) -> String {{
        return socketRead(self, maximum)
    }}
    fn write(self, value: String) -> Number {{
        return socketWrite(self, value)
    }}
    fn close(self) {{ socketClose(self) }}
}}
@native("test.capture")
fn report(value: String)
fn main() {{
    let socket = wait task Socket.connect("127.0.0.1", {port})
    let written = wait task socket.write("ping")
    let response = wait task socket.read(4)
    report(response)
    socket.close()
}}
"#
    );
    let values = run_with_registry(&source, None);
    server.join().unwrap();
    assert_eq!(values, vec![Value::String(KomeString::new("pong"))]);
}

fn run_with_registry(
    source: &str,
    sockets: Option<(
        std::os::unix::net::UnixStream,
        std::os::unix::net::UnixStream,
    )>,
) -> Vec<Value> {
    let values = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::clone(&values);
    let mut registry = builtin_registry();
    registry.register("test.capture", move |arguments: &[Value]| {
        let [value] = arguments else {
            return Err(RuntimeError::native("capture expects one argument"));
        };
        output.lock().unwrap().push(value.clone());
        Ok(Value::Null)
    });
    if let Some((reader, writer)) = sockets {
        registry.register("test.reader_fd", move |_| {
            Ok(Value::Number(Number::from_i64(reader.as_raw_fd() as i64)))
        });
        registry.register("test.writer_fd", move |_| {
            Ok(Value::Number(Number::from_i64(writer.as_raw_fd() as i64)))
        });
    }
    set_thread_registry(registry);
    let module = kome_parser::parse(source).unwrap();
    kome_jit::execute(&module, "main").unwrap();
    clear_thread_registry();
    Arc::try_unwrap(values).unwrap().into_inner().unwrap()
}
