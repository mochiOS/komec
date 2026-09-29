use kome_native_rt::socket::KomeSocket;
use std::os::fd::IntoRawFd;

#[test]
fn closes_the_descriptor_when_the_last_owner_is_dropped() {
    let (left, _right) = std::os::unix::net::UnixStream::pair().unwrap();
    let socket = KomeSocket::from_owned_fd(left.into_raw_fd()).unwrap();
    let copy = socket.clone();
    let fd = copy.fd().unwrap();

    drop(socket);
    assert_ne!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);

    drop(copy);
    assert_eq!(unsafe { libc::fcntl(fd, libc::F_GETFD) }, -1);
    assert_eq!(
        std::io::Error::last_os_error().raw_os_error(),
        Some(libc::EBADF)
    );
}

#[test]
fn close_is_shared_and_idempotent() {
    let (left, _right) = std::os::unix::net::UnixStream::pair().unwrap();
    let socket = KomeSocket::from_owned_fd(left.into_raw_fd()).unwrap();
    let copy = socket.clone();

    socket.close().unwrap();
    socket.close().unwrap();
    assert!(socket.is_closed());
    assert!(copy.is_closed());
    assert_eq!(
        copy.fd().unwrap_err().kind(),
        std::io::ErrorKind::NotConnected
    );
}
