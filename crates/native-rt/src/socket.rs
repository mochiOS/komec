//! Runtime representation of Kome's managed `Socket` type.

use std::fmt;
use std::io;
use std::os::fd::RawFd;
use std::sync::atomic::{AtomicI32, AtomicUsize, Ordering};

const CLOSED_FD: RawFd = -1;

/// A reference-counted owning handle for a non-blocking socket.
///
/// Copies share the descriptor. Closing any copy closes the shared socket;
/// dropping the last copy closes it automatically if necessary.
pub struct KomeSocket {
    raw: u64,
}

struct HeapSocket {
    references: AtomicUsize,
    fd: AtomicI32,
}

impl KomeSocket {
    /// Takes ownership of an open socket descriptor.
    pub fn from_owned_fd(fd: RawFd) -> io::Result<Self> {
        if fd < 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "socket descriptor must be non-negative",
            ));
        }
        let value = Box::new(HeapSocket {
            references: AtomicUsize::new(1),
            fd: AtomicI32::new(fd),
        });
        Ok(Self {
            raw: Box::into_raw(value) as u64,
        })
    }

    /// Returns the live descriptor or an error if the socket is closed.
    pub fn fd(&self) -> io::Result<RawFd> {
        let fd = unsafe { heap_socket(self.raw) }.fd.load(Ordering::Acquire);
        if fd == CLOSED_FD {
            Err(io::Error::new(
                io::ErrorKind::NotConnected,
                "socket is closed",
            ))
        } else {
            Ok(fd)
        }
    }

    /// Closes the shared descriptor. Repeated calls are harmless.
    pub fn close(&self) -> io::Result<()> {
        close_heap(unsafe { heap_socket(self.raw) })
    }

    /// Returns whether the shared descriptor has been closed.
    pub fn is_closed(&self) -> bool {
        unsafe { heap_socket(self.raw) }.fd.load(Ordering::Acquire) == CLOSED_FD
    }

    /// Returns the raw runtime representation without transferring ownership.
    pub const fn raw(&self) -> u64 {
        self.raw
    }

    /// Transfers ownership into the raw runtime representation.
    pub fn into_raw(self) -> u64 {
        let raw = self.raw;
        std::mem::forget(self);
        raw
    }

    /// Creates an owned socket from a raw runtime representation.
    ///
    /// The returned value acquires an additional reference.
    ///
    /// # Safety
    ///
    /// `raw` must be a valid live Kome `Socket` runtime handle.
    pub unsafe fn from_raw_retain(raw: u64) -> Self {
        unsafe { retain(raw) };
        Self { raw }
    }
}

impl Clone for KomeSocket {
    fn clone(&self) -> Self {
        unsafe { retain(self.raw) };
        Self { raw: self.raw }
    }
}

impl Drop for KomeSocket {
    fn drop(&mut self) {
        unsafe { release(self.raw) };
    }
}

impl PartialEq for KomeSocket {
    fn eq(&self, other: &Self) -> bool {
        self.raw == other.raw
    }
}

impl Eq for KomeSocket {}

impl fmt::Debug for KomeSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("KomeSocket")
            .field("fd", &self.fd().ok())
            .finish()
    }
}

impl fmt::Display for KomeSocket {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.fd() {
            Ok(fd) => write!(formatter, "Socket({fd})"),
            Err(_) => formatter.write_str("Socket(closed)"),
        }
    }
}

/// Increments the reference count for a raw Kome `Socket` handle.
///
/// # Safety
///
/// `raw` must be a valid live Kome `Socket` runtime handle.
pub unsafe fn retain(raw: u64) {
    unsafe { heap_socket(raw) }
        .references
        .fetch_add(1, Ordering::Relaxed);
}

/// Decrements the reference count and closes the final socket owner.
///
/// # Safety
///
/// `raw` must be a valid Kome `Socket` handle and the caller must own one
/// reference to it.
pub unsafe fn release(raw: u64) {
    let socket = unsafe { heap_socket(raw) };
    if socket.references.fetch_sub(1, Ordering::AcqRel) != 1 {
        return;
    }
    let pointer = raw as *mut HeapSocket;
    let socket = unsafe { Box::from_raw(pointer) };
    let _ = close_heap(&socket);
}

fn close_heap(socket: &HeapSocket) -> io::Result<()> {
    let fd = socket.fd.swap(CLOSED_FD, Ordering::AcqRel);
    if fd == CLOSED_FD {
        return Ok(());
    }
    if unsafe { libc::close(fd) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

unsafe fn heap_socket(raw: u64) -> &'static HeapSocket {
    unsafe { &*(raw as *const HeapSocket) }
}

/// Increments the reference count for a raw Kome `Socket` handle.
///
/// # Safety
///
/// `raw` must be a valid live Kome `Socket` runtime handle.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_socket_retain(raw: u64) {
    unsafe { retain(raw) };
}

/// Decrements the reference count for a raw Kome `Socket` handle.
///
/// # Safety
///
/// `raw` must be a valid Kome `Socket` handle and the caller must own one
/// reference to it.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_socket_release(raw: u64) {
    unsafe { release(raw) };
}
