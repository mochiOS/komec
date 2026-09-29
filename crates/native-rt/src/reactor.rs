//! Linux `epoll` reactor used by the cooperative task scheduler.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io;
use std::os::fd::RawFd;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

const WAKE_TOKEN: u64 = u64::MAX;
static WAKE_FDS: OnceLock<Mutex<Vec<RawFd>>> = OnceLock::new();

#[derive(Default)]
struct Waiters {
    readers: Vec<u64>,
    writers: Vec<u64>,
}

struct Reactor {
    epoll: RawFd,
    wake: RawFd,
    waiters: HashMap<RawFd, Waiters>,
}

impl Reactor {
    fn new() -> Self {
        let epoll = unsafe { libc::epoll_create1(libc::EPOLL_CLOEXEC) };
        assert!(epoll >= 0, "failed to create the Kome epoll reactor");
        let wake = unsafe { libc::eventfd(0, libc::EFD_CLOEXEC | libc::EFD_NONBLOCK) };
        assert!(wake >= 0, "failed to create the Kome reactor eventfd");
        let mut event = libc::epoll_event {
            events: libc::EPOLLIN as u32,
            u64: WAKE_TOKEN,
        };
        let status = unsafe { libc::epoll_ctl(epoll, libc::EPOLL_CTL_ADD, wake, &mut event) };
        assert_eq!(status, 0, "failed to register the Kome reactor eventfd");
        WAKE_FDS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .push(wake);
        Self {
            epoll,
            wake,
            waiters: HashMap::new(),
        }
    }

    fn register(&mut self, fd: RawFd, task: u64, writable: bool) -> io::Result<()> {
        let waiters = self.waiters.entry(fd).or_default();
        let tasks = if writable {
            &mut waiters.writers
        } else {
            &mut waiters.readers
        };
        if !tasks.contains(&task) {
            tasks.push(task);
        }
        self.update(fd)
    }

    fn update(&mut self, fd: RawFd) -> io::Result<()> {
        let Some(waiters) = self.waiters.get(&fd) else {
            return Ok(());
        };
        let mut events = libc::EPOLLERR | libc::EPOLLHUP | libc::EPOLLRDHUP;
        if !waiters.readers.is_empty() {
            events |= libc::EPOLLIN;
        }
        if !waiters.writers.is_empty() {
            events |= libc::EPOLLOUT;
        }
        let mut event = libc::epoll_event {
            events: events as u32,
            u64: fd as u64,
        };
        let mut status =
            unsafe { libc::epoll_ctl(self.epoll, libc::EPOLL_CTL_MOD, fd, &mut event) };
        if status != 0 && io::Error::last_os_error().raw_os_error() == Some(libc::ENOENT) {
            status = unsafe { libc::epoll_ctl(self.epoll, libc::EPOLL_CTL_ADD, fd, &mut event) };
        }
        if status == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        }
    }

    fn unregister_task(&mut self, task: u64) {
        let fds = self.waiters.keys().copied().collect::<Vec<_>>();
        for fd in fds {
            let empty = if let Some(waiters) = self.waiters.get_mut(&fd) {
                waiters.readers.retain(|waiter| *waiter != task);
                waiters.writers.retain(|waiter| *waiter != task);
                waiters.readers.is_empty() && waiters.writers.is_empty()
            } else {
                false
            };
            if empty {
                self.waiters.remove(&fd);
                unsafe {
                    libc::epoll_ctl(self.epoll, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut())
                };
            } else {
                let _ = self.update(fd);
            }
        }
    }

    fn poll(&mut self, timeout: Option<Duration>) -> io::Result<Vec<u64>> {
        let milliseconds = timeout.map_or(-1, |duration| {
            duration
                .as_millis()
                .max(u128::from(!duration.is_zero()))
                .min(i32::MAX as u128) as i32
        });
        let mut events = [libc::epoll_event { events: 0, u64: 0 }; 32];
        let count = unsafe {
            libc::epoll_wait(
                self.epoll,
                events.as_mut_ptr(),
                events.len() as i32,
                milliseconds,
            )
        };
        if count < 0 {
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                return Ok(Vec::new());
            }
            return Err(error);
        }
        let mut ready = Vec::new();
        for event in events.into_iter().take(count as usize) {
            if event.u64 == WAKE_TOKEN {
                let mut value = 0_u64;
                unsafe {
                    libc::read(
                        self.wake,
                        (&mut value as *mut u64).cast(),
                        std::mem::size_of::<u64>(),
                    );
                }
                continue;
            }
            let fd = event.u64 as RawFd;
            let Some(waiters) = self.waiters.get_mut(&fd) else {
                continue;
            };
            let error =
                event.events & (libc::EPOLLERR | libc::EPOLLHUP | libc::EPOLLRDHUP) as u32 != 0;
            if error || event.events & libc::EPOLLIN as u32 != 0 {
                ready.append(&mut waiters.readers);
            }
            if error || event.events & libc::EPOLLOUT as u32 != 0 {
                ready.append(&mut waiters.writers);
            }
            if waiters.readers.is_empty() && waiters.writers.is_empty() {
                self.waiters.remove(&fd);
                unsafe {
                    libc::epoll_ctl(self.epoll, libc::EPOLL_CTL_DEL, fd, std::ptr::null_mut())
                };
            } else {
                self.update(fd)?;
            }
        }
        ready.sort_unstable();
        ready.dedup();
        Ok(ready)
    }
}

impl Drop for Reactor {
    fn drop(&mut self) {
        WAKE_FDS
            .get_or_init(|| Mutex::new(Vec::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .retain(|fd| *fd != self.wake);
        unsafe {
            libc::close(self.wake);
            libc::close(self.epoll);
        }
    }
}

thread_local! {
    static REACTOR: RefCell<Reactor> = RefCell::new(Reactor::new());
}

pub(crate) fn register(fd: RawFd, task: u64, writable: bool) -> io::Result<()> {
    REACTOR.with(|reactor| reactor.borrow_mut().register(fd, task, writable))
}

pub(crate) fn unregister_task(task: u64) {
    REACTOR.with(|reactor| reactor.borrow_mut().unregister_task(task));
}

pub(crate) fn poll(timeout: Option<Duration>) -> io::Result<Vec<u64>> {
    REACTOR.with(|reactor| reactor.borrow_mut().poll(timeout))
}

pub(crate) fn notify() {
    let wake_fds = WAKE_FDS
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    for wake in wake_fds.iter().copied() {
        let value = 1_u64;
        unsafe {
            libc::write(
                wake,
                (&value as *const u64).cast(),
                std::mem::size_of::<u64>(),
            );
        }
    }
}
