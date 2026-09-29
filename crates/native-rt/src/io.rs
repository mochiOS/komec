//! Cooperative Linux timer and socket operations.

use crate::string::KomeString;
use crate::task::{__kome_task_sleep, __kome_task_wait_readable, __kome_task_wait_writable};
use std::io;
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
