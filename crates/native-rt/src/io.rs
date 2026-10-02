//! Cooperative Linux timer and socket operations.

use crate::socket::KomeSocket;
use crate::string::KomeString;
use crate::task::{__kome_task_sleep, __kome_task_wait_readable, __kome_task_wait_writable};
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::os::fd::RawFd;

const MAX_READ_SIZE: usize = 16 * 1024 * 1024;

fn set_nonblocking(fd: RawFd) -> io::Result<()> {
    let flags = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if flags < 0 {
        return Err(io::Error::last_os_error());
    }
    if flags & libc::O_NONBLOCK == 0
        && unsafe { libc::fcntl(fd, libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Suspends the current task until the requested millisecond delay expires.
pub fn sleep(milliseconds: u64) {
    __kome_task_sleep(milliseconds);
}

/// Opens a non-blocking TCP socket and cooperatively waits for connection.
///
/// `host` must be a numeric IPv4 or IPv6 address. DNS resolution is kept out
/// of this readiness layer so it never blocks the cooperative scheduler.
pub fn socket_connect(host: &str, port: u16) -> io::Result<KomeSocket> {
    let address = host.parse::<IpAddr>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "socket host must be a numeric IPv4 or IPv6 address",
        )
    })?;
    let domain = match address {
        IpAddr::V4(_) => libc::AF_INET,
        IpAddr::V6(_) => libc::AF_INET6,
    };
    let fd = unsafe {
        libc::socket(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = KomeSocket::from_owned_fd(fd)?;
    let status = match address {
        IpAddr::V4(address) => connect_v4(fd, address, port),
        IpAddr::V6(address) => connect_v6(fd, address, port),
    };
    if status == 0 {
        return Ok(socket);
    }
    let error = io::Error::last_os_error();
    if error.raw_os_error() != Some(libc::EINPROGRESS) {
        return Err(error);
    }
    let status = __kome_task_wait_writable(fd);
    if status < 0 {
        return Err(io::Error::from_raw_os_error(-status));
    }
    let mut socket_error = 0_i32;
    let mut length = std::mem::size_of::<i32>() as libc::socklen_t;
    let status = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_ERROR,
            (&mut socket_error as *mut i32).cast(),
            &mut length,
        )
    };
    if status < 0 {
        return Err(io::Error::last_os_error());
    }
    if socket_error != 0 {
        return Err(io::Error::from_raw_os_error(socket_error));
    }
    Ok(socket)
}

/// Opens a non-blocking TCP listener on a numeric IPv4 or IPv6 address.
pub fn socket_bind(host: &str, port: u16) -> io::Result<KomeSocket> {
    let address = host.parse::<IpAddr>().map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "listener host must be a numeric IPv4 or IPv6 address",
        )
    })?;
    let domain = match address {
        IpAddr::V4(_) => libc::AF_INET,
        IpAddr::V6(_) => libc::AF_INET6,
    };
    let fd = unsafe {
        libc::socket(
            domain,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }
    let socket = KomeSocket::from_owned_fd(fd)?;
    let reuse = 1_i32;
    let status = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            (&reuse as *const i32).cast(),
            std::mem::size_of_val(&reuse) as libc::socklen_t,
        )
    };
    if status < 0 {
        return Err(io::Error::last_os_error());
    }
    let status = match address {
        IpAddr::V4(address) => bind_v4(fd, address, port),
        IpAddr::V6(address) => bind_v6(fd, address, port),
    };
    if status < 0 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::listen(fd, 128) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(socket)
}

/// Cooperatively waits for and accepts one connection from a TCP listener.
pub fn managed_socket_accept(listener: &KomeSocket) -> io::Result<KomeSocket> {
    let fd = listener.fd()?;
    loop {
        let accepted = unsafe {
            libc::accept4(
                fd,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            )
        };
        if accepted >= 0 {
            return KomeSocket::from_owned_fd(accepted);
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::Interrupted => continue,
            io::ErrorKind::WouldBlock => {
                let status = __kome_task_wait_readable(fd);
                if status < 0 {
                    return Err(io::Error::from_raw_os_error(-status));
                }
            }
            _ => return Err(error),
        }
    }
}

fn connect_v4(fd: RawFd, address: Ipv4Addr, port: u16) -> i32 {
    let address = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(address.octets()),
        },
        sin_zero: [0; 8],
    };
    unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_in).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    }
}

fn connect_v6(fd: RawFd, address: Ipv6Addr, port: u16) -> i32 {
    let address = libc::sockaddr_in6 {
        sin6_family: libc::AF_INET6 as libc::sa_family_t,
        sin6_port: port.to_be(),
        sin6_flowinfo: 0,
        sin6_addr: libc::in6_addr {
            s6_addr: address.octets(),
        },
        sin6_scope_id: 0,
    };
    unsafe {
        libc::connect(
            fd,
            (&address as *const libc::sockaddr_in6).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    }
}

fn bind_v4(fd: RawFd, address: Ipv4Addr, port: u16) -> i32 {
    let address = libc::sockaddr_in {
        sin_family: libc::AF_INET as libc::sa_family_t,
        sin_port: port.to_be(),
        sin_addr: libc::in_addr {
            s_addr: u32::from_ne_bytes(address.octets()),
        },
        sin_zero: [0; 8],
    };
    unsafe {
        libc::bind(
            fd,
            (&address as *const libc::sockaddr_in).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    }
}

fn bind_v6(fd: RawFd, address: Ipv6Addr, port: u16) -> i32 {
    let address = libc::sockaddr_in6 {
        sin6_family: libc::AF_INET6 as libc::sa_family_t,
        sin6_port: port.to_be(),
        sin6_flowinfo: 0,
        sin6_addr: libc::in6_addr {
            s6_addr: address.octets(),
        },
        sin6_scope_id: 0,
    };
    unsafe {
        libc::bind(
            fd,
            (&address as *const libc::sockaddr_in6).cast(),
            std::mem::size_of_val(&address) as libc::socklen_t,
        )
    }
}

/// Reads UTF-8 data from a managed socket.
pub fn managed_socket_read(socket: &KomeSocket, maximum: usize) -> io::Result<KomeString> {
    socket_read(socket.fd()?, maximum)
}

/// Writes all UTF-8 data to a managed socket.
pub fn managed_socket_write(socket: &KomeSocket, value: &KomeString) -> io::Result<usize> {
    socket_write(socket.fd()?, value)
}

/// Reads up to `maximum` bytes from a non-blocking socket.
///
/// An `EAGAIN` result suspends the current task through the reactor and retries
/// after epoll reports readability.
pub fn socket_read(fd: RawFd, maximum: usize) -> io::Result<KomeString> {
    if maximum == 0 || maximum > MAX_READ_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("socket read size must be between 1 and {MAX_READ_SIZE}"),
        ));
    }
    set_nonblocking(fd)?;
    let mut buffer = vec![0_u8; maximum];
    loop {
        let count = unsafe { libc::read(fd, buffer.as_mut_ptr().cast(), buffer.len()) };
        if count >= 0 {
            buffer.truncate(count as usize);
            let value = String::from_utf8(buffer)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error.utf8_error()))?;
            return Ok(KomeString::new(value));
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::Interrupted => continue,
            io::ErrorKind::WouldBlock => {
                let status = __kome_task_wait_readable(fd);
                if status < 0 {
                    if status == -libc::ECANCELED {
                        return Ok(KomeString::new(""));
                    }
                    return Err(io::Error::from_raw_os_error(-status));
                }
            }
            _ => return Err(error),
        }
    }
}

/// Writes all UTF-8 bytes to a non-blocking socket.
///
/// An `EAGAIN` result suspends the current task through the reactor and retries
/// after epoll reports writability.
pub fn socket_write(fd: RawFd, value: &KomeString) -> io::Result<usize> {
    set_nonblocking(fd)?;
    let bytes = value.as_str().as_bytes();
    let mut written = 0;
    while written < bytes.len() {
        let count =
            unsafe { libc::write(fd, bytes[written..].as_ptr().cast(), bytes.len() - written) };
        if count > 0 {
            written += count as usize;
            continue;
        }
        if count == 0 {
            return Err(io::Error::new(
                io::ErrorKind::WriteZero,
                "socket write returned zero bytes",
            ));
        }
        let error = io::Error::last_os_error();
        match error.kind() {
            io::ErrorKind::Interrupted => continue,
            io::ErrorKind::WouldBlock => {
                let status = __kome_task_wait_writable(fd);
                if status < 0 {
                    if status == -libc::ECANCELED {
                        return Ok(written);
                    }
                    return Err(io::Error::from_raw_os_error(-status));
                }
            }
            _ => return Err(error),
        }
    }
    Ok(written)
}

/// C ABI wrapper for cooperative sleeping.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_io_sleep(milliseconds: u64) {
    sleep(milliseconds);
}

/// Reads UTF-8 data and transfers the resulting Kome string to the caller.
///
/// Returns `0` after an I/O or UTF-8 error.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_socket_read(fd: RawFd, maximum: usize) -> u64 {
    socket_read(fd, maximum).map_or(0, KomeString::into_raw)
}

/// Writes a raw Kome string to a socket, returning the byte count or `-1`.
///
/// # Safety
///
/// `value` must be a valid live Kome string handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_socket_write(fd: RawFd, value: u64) -> i64 {
    let value = unsafe { KomeString::from_raw_retain(value) };
    socket_write(fd, &value).map_or(-1, |written| written as i64)
}
