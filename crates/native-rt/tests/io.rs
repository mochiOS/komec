use kome_native_rt::io::{
    managed_socket_accept, sleep, socket_bind, socket_connect, socket_read, socket_write,
};
use kome_native_rt::string::{KomeString, release};
use kome_native_rt::task::{
    __kome_task_dealloc, __kome_task_release, __kome_task_result, __kome_task_spawn,
    __kome_task_state, __kome_task_wait, TASK_COMPLETED, TASK_SUSPENDED,
};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

static SLEEPER: AtomicU64 = AtomicU64::new(0);

unsafe extern "C" fn sleeping_task(_task: u64, _captures: *const u64, execute: u8) -> u64 {
    if execute == 0 {
        return 0;
    }
    sleep(5);
    42
}

unsafe extern "C" fn observes_sleep(_task: u64, _captures: *const u64, execute: u8) -> u64 {
    if execute == 0 {
        return 0;
    }
    assert_eq!(
        unsafe { __kome_task_state(SLEEPER.load(Ordering::SeqCst)) },
        TASK_SUSPENDED
    );
    1
}

unsafe extern "C" fn reads_socket(_task: u64, captures: *const u64, execute: u8) -> u64 {
    if execute == 0 {
        return 0;
    }
    let fd = unsafe { *captures } as i32;
    socket_read(fd, 64).unwrap().into_raw()
}

unsafe extern "C" fn writes_socket(_task: u64, captures: *const u64, execute: u8) -> u64 {
    if execute == 0 {
        return 0;
    }
    let fd = unsafe { *captures } as i32;
    let reader = unsafe { *captures.add(1) };
    assert_eq!(unsafe { __kome_task_state(reader) }, TASK_SUSPENDED);
    socket_write(fd, &KomeString::new("ready")).unwrap() as u64
}

#[test]
fn sleep_suspends_without_preventing_another_task_from_running() {
    let captures: [u64; 0] = [];
    let started = Instant::now();
    let sleeper = unsafe { __kome_task_spawn(sleeping_task, captures.as_ptr(), 0) };
    SLEEPER.store(sleeper, Ordering::SeqCst);
    let observer = unsafe { __kome_task_spawn(observes_sleep, captures.as_ptr(), 0) };
    unsafe {
        assert_eq!(__kome_task_wait(observer), TASK_COMPLETED);
        assert_eq!(__kome_task_wait(sleeper), TASK_COMPLETED);
        assert_eq!(__kome_task_result(sleeper), 42);
        assert!(started.elapsed() >= Duration::from_millis(5));
        assert_eq!(__kome_task_release(observer), 1);
        __kome_task_dealloc(observer);
        assert_eq!(__kome_task_release(sleeper), 1);
        __kome_task_dealloc(sleeper);
    }
}

#[test]
fn socket_read_suspends_until_epoll_reports_readiness() {
    let (reader_socket, writer_socket) = std::os::unix::net::UnixStream::pair().unwrap();
    let reader_captures = [reader_socket.as_raw_fd() as u64];
    let reader = unsafe {
        __kome_task_spawn(
            reads_socket,
            reader_captures.as_ptr(),
            reader_captures.len(),
        )
    };
    let writer_captures = [writer_socket.as_raw_fd() as u64, reader];
    let writer = unsafe {
        __kome_task_spawn(
            writes_socket,
            writer_captures.as_ptr(),
            writer_captures.len(),
        )
    };
    unsafe {
        assert_eq!(__kome_task_wait(reader), TASK_COMPLETED);
        assert_eq!(__kome_task_wait(writer), TASK_COMPLETED);
        let raw = __kome_task_result(reader);
        let value = KomeString::from_raw_retain(raw);
        assert_eq!(value.as_str(), "ready");
        release(raw);
        assert_eq!(__kome_task_release(reader), 1);
        __kome_task_dealloc(reader);
        assert_eq!(__kome_task_release(writer), 1);
        __kome_task_dealloc(writer);
    }
}

#[test]
fn managed_socket_connects_to_a_tcp_listener() {
    let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || listener.accept().unwrap());

    let socket = socket_connect("127.0.0.1", port).unwrap();
    assert!(!socket.is_closed());
    socket.close().unwrap();
    let _ = server.join().unwrap();
}

#[test]
fn managed_listener_accepts_a_tcp_connection() {
    let probe = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
    let port = probe.local_addr().unwrap().port();
    drop(probe);

    let listener = socket_bind("127.0.0.1", port).unwrap();
    let client = std::thread::spawn(move || std::net::TcpStream::connect(("127.0.0.1", port)));

    let accepted = managed_socket_accept(&listener).unwrap();
    assert!(!accepted.is_closed());
    accepted.close().unwrap();
    listener.close().unwrap();
    let _ = client.join().unwrap();
}
