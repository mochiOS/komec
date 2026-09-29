//! Cooperative, reference-counted task runtime and its stable C ABI.
//!
//! Spawned tasks own a user-space stack. Waiting saves the current context,
//! registers a waiter, and yields to the thread-local scheduler.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::io::Write;
use std::mem::MaybeUninit;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

static NEXT_TASK_ID: AtomicUsize = AtomicUsize::new(1);
static NEXT_COMPLETION: AtomicU64 = AtomicU64::new(1);
static SCHEDULER_WAKE: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();
const TASK_STACK_SIZE: usize = 1024 * 1024;

/// Stable identifier assigned to one runtime task.
pub type TaskId = u64;

type TaskEntry = unsafe extern "C" fn(u64, *const u64, u8) -> u64;

/// Numeric ABI value for [`TaskState::Pending`].
pub const TASK_PENDING: u8 = 0;
/// Numeric ABI value for [`TaskState::Running`].
pub const TASK_RUNNING: u8 = 1;
/// Numeric ABI value for [`TaskState::Completed`].
pub const TASK_COMPLETED: u8 = 2;
/// Numeric ABI value for [`TaskState::Cancelled`].
pub const TASK_CANCELLED: u8 = 3;
/// Numeric ABI value for [`TaskState::Failed`].
pub const TASK_FAILED: u8 = 4;
/// Numeric ABI value for [`TaskState::CancellationRequested`].
pub const TASK_CANCELLATION_REQUESTED: u8 = 5;
/// Numeric ABI value for [`TaskState::Suspended`].
pub const TASK_SUSPENDED: u8 = 6;
/// Internal runnable state used after wakeup.
pub const TASK_RUNNABLE: u8 = 7;

/// Observable lifecycle state of a runtime task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskState {
    Pending = TASK_PENDING,
    Running = TASK_RUNNING,
    Completed = TASK_COMPLETED,
    Cancelled = TASK_CANCELLED,
    Failed = TASK_FAILED,
    CancellationRequested = TASK_CANCELLATION_REQUESTED,
    Suspended = TASK_SUSPENDED,
    Runnable = TASK_RUNNABLE,
}

impl TaskState {
    fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Cancelled | Self::Failed)
    }

    fn is_active(self) -> bool {
        !self.is_terminal()
    }
}

/// Result stored by a task after it reaches a terminal state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskResult {
    Completed(u64),
    Cancelled,
    Failed(u64),
}

struct Fiber {
    context: Box<libc::ucontext_t>,
    _stack: Box<[u8]>,
}

unsafe impl Send for Fiber {}

struct TaskData {
    state: TaskState,
    result: Option<TaskResult>,
    completion_order: u64,
    waiters: Vec<u64>,
    entry: Option<TaskEntry>,
    captures: Option<Vec<u64>>,
    fiber: Option<Fiber>,
}

struct TaskRecord {
    id: TaskId,
    references: AtomicUsize,
    data: Mutex<TaskData>,
    ready: Condvar,
}

struct Timer {
    deadline: Instant,
    waiter: u64,
    target: u64,
}

struct Scheduler {
    context: Box<libc::ucontext_t>,
    current: Option<u64>,
    runnable: VecDeque<u64>,
    timers: Vec<Timer>,
}

impl Scheduler {
    fn new() -> Self {
        Self {
            context: Box::new(unsafe { MaybeUninit::zeroed().assume_init() }),
            current: None,
            runnable: VecDeque::new(),
            timers: Vec::new(),
        }
    }
}

thread_local! {
    static SCHEDULER: RefCell<Scheduler> = RefCell::new(Scheduler::new());
}

unsafe fn record(handle: u64) -> &'static TaskRecord {
    unsafe { &*(handle as *const TaskRecord) }
}

fn state(handle: u64) -> TaskState {
    let task = unsafe { record(handle) };
    task.data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .state
}

fn scheduler_signal() {
    let (generation, ready) = SCHEDULER_WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()));
    let mut generation = generation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *generation = generation.wrapping_add(1);
    ready.notify_all();
}

fn enqueue(handle: u64) {
    SCHEDULER.with(|scheduler| {
        let mut scheduler = scheduler.borrow_mut();
        if !scheduler.runnable.contains(&handle) {
            scheduler.runnable.push_back(handle);
        }
    });
    scheduler_signal();
}

fn remove_runnable(handle: u64) {
    SCHEDULER.with(|scheduler| {
        scheduler
            .borrow_mut()
            .runnable
            .retain(|queued| *queued != handle);
    });
}

fn current_task() -> Option<u64> {
    SCHEDULER.with(|scheduler| scheduler.borrow().current)
}

fn wake_waiters(waiters: Vec<u64>) {
    for waiter in waiters {
        unsafe { __kome_task_wake(waiter) };
    }
    scheduler_signal();
}

fn terminal_transition(handle: u64, result: TaskResult, next: TaskState) {
    let task = unsafe { record(handle) };
    let waiters = {
        let mut data = task
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if data.state.is_terminal() {
            return;
        }
        data.result = Some(result);
        data.state = next;
        if next == TaskState::Completed {
            data.completion_order = NEXT_COMPLETION.fetch_add(1, Ordering::Relaxed);
        }
        std::mem::take(&mut data.waiters)
    };
    task.ready.notify_all();
    wake_waiters(waiters);
}

unsafe extern "C" fn task_trampoline(handle: usize) {
    let handle = handle as u64;
    let task = unsafe { record(handle) };
    let (entry, captures) = {
        let mut data = task
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        (
            data.entry.expect("spawned task has an entry point"),
            data.captures.take().unwrap_or_default(),
        )
    };
    let result = unsafe { entry(handle, captures.as_ptr(), 1) };
    if state(handle) == TaskState::CancellationRequested {
        terminal_transition(handle, TaskResult::Cancelled, TaskState::Cancelled);
    } else {
        unsafe { __kome_task_complete(handle, result) };
    }
}

fn make_fiber(handle: u64) -> Fiber {
    let mut context: Box<libc::ucontext_t> =
        Box::new(unsafe { MaybeUninit::zeroed().assume_init() });
    let mut stack = vec![0_u8; TASK_STACK_SIZE].into_boxed_slice();
    unsafe {
        libc::getcontext(context.as_mut());
        context.uc_stack.ss_sp = stack.as_mut_ptr().cast();
        context.uc_stack.ss_size = stack.len();
        context.uc_stack.ss_flags = 0;
        context.uc_link = SCHEDULER
            .with(|scheduler| scheduler.borrow_mut().context.as_mut() as *mut libc::ucontext_t);
        let trampoline: extern "C" fn() =
            std::mem::transmute::<unsafe extern "C" fn(usize), extern "C" fn()>(task_trampoline);
        libc::makecontext(context.as_mut(), trampoline, 1, handle as usize);
    }
    Fiber {
        context,
        _stack: stack,
    }
}

fn run_one() -> bool {
    process_timers();
    let Some(handle) = SCHEDULER.with(|scheduler| scheduler.borrow_mut().runnable.pop_front())
    else {
        return false;
    };
    let context = {
        let task = unsafe { record(handle) };
        let mut data = task
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if !matches!(data.state, TaskState::Pending | TaskState::Runnable) {
            return true;
        }
        data.state = TaskState::Running;
        data.fiber
            .as_mut()
            .map(|fiber| fiber.context.as_mut() as *mut libc::ucontext_t)
    };
    let Some(context) = context else {
        return true;
    };
    let scheduler_context = SCHEDULER.with(|scheduler| {
        let mut scheduler = scheduler.borrow_mut();
        scheduler.current = Some(handle);
        scheduler.context.as_mut() as *mut libc::ucontext_t
    });
    unsafe { libc::swapcontext(scheduler_context, context) };
    SCHEDULER.with(|scheduler| scheduler.borrow_mut().current = None);
    process_timers();
    true
}

fn suspend_current() {
    let handle = current_task().expect("only a running task can suspend");
    let task_context = {
        let task = unsafe { record(handle) };
        let mut data = task
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        data.state = TaskState::Suspended;
        data.fiber
            .as_mut()
            .expect("running task has a fiber")
            .context
            .as_mut() as *mut libc::ucontext_t
    };
    let scheduler_context = SCHEDULER
        .with(|scheduler| scheduler.borrow_mut().context.as_mut() as *mut libc::ucontext_t);
    unsafe { libc::swapcontext(task_context, scheduler_context) };
}

fn register_waiter(target: u64, waiter: u64) -> bool {
    let task = unsafe { record(target) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if data.state.is_terminal() {
        return false;
    }
    if !data.waiters.contains(&waiter) {
        data.waiters.push(waiter);
    }
    true
}

fn unregister_waiter(target: u64, waiter: u64) {
    unsafe { record(target) }
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .waiters
        .retain(|registered| *registered != waiter);
}

fn nearest_timer() -> Option<Instant> {
    SCHEDULER.with(|scheduler| {
        scheduler
            .borrow()
            .timers
            .iter()
            .map(|timer| timer.deadline)
            .min()
    })
}

fn process_timers() {
    let now = Instant::now();
    let expired = SCHEDULER.with(|scheduler| {
        let mut scheduler = scheduler.borrow_mut();
        let timers = std::mem::take(&mut scheduler.timers);
        let (expired, pending) = timers.into_iter().partition(|timer| timer.deadline <= now);
        scheduler.timers = pending;
        expired
    });
    for timer in expired {
        unregister_waiter(timer.target, timer.waiter);
        if state(timer.waiter) == TaskState::Suspended {
            unsafe { __kome_task_cancel(timer.target) };
            unsafe { __kome_task_wake(timer.waiter) };
        }
    }
}

fn park_scheduler(deadline: Option<Instant>) {
    let (generation, ready) = SCHEDULER_WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()));
    let generation = generation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(deadline) = deadline {
        let duration = deadline.saturating_duration_since(Instant::now());
        drop(
            ready
                .wait_timeout(generation, duration)
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
    } else {
        drop(
            ready
                .wait(generation)
                .unwrap_or_else(|poisoned| poisoned.into_inner()),
        );
    }
}

fn drive_until(handle: u64, deadline: Option<Instant>) -> TaskState {
    loop {
        let current = state(handle);
        if current.is_terminal() {
            return current;
        }
        if deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            unsafe { __kome_task_cancel(handle) };
            return state(handle);
        }
        if !run_one() {
            let wake = match (deadline, nearest_timer()) {
                (Some(left), Some(right)) => Some(left.min(right)),
                (Some(value), None) | (None, Some(value)) => Some(value),
                (None, None) => None,
            };
            park_scheduler(wake);
            process_timers();
        }
    }
}

/// Allocates a pending manual task with one owning reference.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_task_create() -> u64 {
    Box::into_raw(Box::new(TaskRecord {
        id: NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed) as TaskId,
        references: AtomicUsize::new(1),
        data: Mutex::new(TaskData {
            state: TaskState::Pending,
            result: None,
            completion_order: 0,
            waiters: Vec::new(),
            entry: None,
            captures: None,
            fiber: None,
        }),
        ready: Condvar::new(),
    })) as u64
}

/// Creates a runnable task by copying raw capture slots.
///
/// # Safety
///
/// `entry` must use the generated task ABI and `captures` must contain
/// `capture_count` readable slots during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_spawn(
    entry: TaskEntry,
    captures: *const u64,
    capture_count: usize,
) -> u64 {
    let handle = __kome_task_create();
    let captures = if capture_count == 0 {
        Vec::new()
    } else {
        unsafe { std::slice::from_raw_parts(captures, capture_count) }.to_vec()
    };
    let fiber = make_fiber(handle);
    let task = unsafe { record(handle) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    data.entry = Some(entry);
    data.captures = Some(captures);
    data.fiber = Some(fiber);
    drop(data);
    enqueue(handle);
    handle
}

/// Transitions a pending manual task to running.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_start(handle: u64) {
    let task = unsafe { record(handle) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if data.state == TaskState::Pending {
        data.state = TaskState::Running;
    }
}

/// Stores a successful result and wakes every waiter.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_complete(handle: u64, result: u64) {
    terminal_transition(handle, TaskResult::Completed(result), TaskState::Completed);
}

/// Stores an error code and wakes every waiter.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_fail(handle: u64, error: u64) {
    terminal_transition(handle, TaskResult::Failed(error), TaskState::Failed);
}

/// Requests cancellation, immediately cancelling a task not yet started.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_cancel(handle: u64) {
    let task = unsafe { record(handle) };
    let cleanup = {
        let mut data = task
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        match data.state {
            TaskState::Pending | TaskState::Runnable => {
                data.state = TaskState::Cancelled;
                data.result = Some(TaskResult::Cancelled);
                data.entry.zip(data.captures.take())
            }
            TaskState::Running | TaskState::Suspended => {
                data.state = TaskState::CancellationRequested;
                None
            }
            TaskState::CancellationRequested
            | TaskState::Completed
            | TaskState::Failed
            | TaskState::Cancelled => None,
        }
    };
    if let Some((entry, captures)) = cleanup {
        remove_runnable(handle);
        unsafe { entry(handle, captures.as_ptr(), 0) };
        let waiters = {
            let mut data = task
                .data
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            std::mem::take(&mut data.waiters)
        };
        task.ready.notify_all();
        wake_waiters(waiters);
    } else {
        scheduler_signal();
    }
}

/// Moves a suspended task back to the runtime runnable queue.
///
/// # Safety
///
/// `handle` must identify a live task on this scheduler thread.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_wake(handle: u64) {
    let task = unsafe { record(handle) };
    let should_enqueue = {
        let mut data = task
            .data
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if data.state == TaskState::Suspended {
            data.state = TaskState::Runnable;
            true
        } else {
            false
        }
    };
    if should_enqueue {
        enqueue(handle);
    }
}

/// Returns a task's current lifecycle state.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_state(handle: u64) -> u8 {
    state(handle) as u8
}

/// Waits cooperatively until a task reaches a terminal state.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_wait(handle: u64) -> u8 {
    if let Some(waiter) = current_task() {
        loop {
            let current = state(handle);
            if current.is_terminal() {
                return current as u8;
            }
            if register_waiter(handle, waiter) {
                suspend_current();
                unregister_waiter(handle, waiter);
            }
        }
    }
    drive_until(handle, None) as u8
}

/// Waits cooperatively until completion or a millisecond deadline.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_wait_timeout(handle: u64, milliseconds: u64) -> u8 {
    let deadline = Instant::now() + Duration::from_millis(milliseconds);
    if let Some(waiter) = current_task() {
        let current = state(handle);
        if current.is_terminal() {
            return current as u8;
        }
        if register_waiter(handle, waiter) {
            SCHEDULER.with(|scheduler| {
                scheduler.borrow_mut().timers.push(Timer {
                    deadline,
                    waiter,
                    target: handle,
                });
            });
            suspend_current();
            unregister_waiter(handle, waiter);
        }
        return state(handle) as u8;
    }
    drive_until(handle, Some(deadline)) as u8
}

/// Waits for and returns the first successfully completed task index.
///
/// # Safety
///
/// `handles` must point to `length` live task handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_race(handles: *const u64, length: usize) -> usize {
    let handles = unsafe { std::slice::from_raw_parts(handles, length) };
    loop {
        let mut winner = None;
        let mut active = false;
        for (index, handle) in handles.iter().copied().enumerate() {
            let task = unsafe { record(handle) };
            let data = task
                .data
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            match data.state {
                TaskState::Completed => {
                    if winner.is_none_or(|(_, order)| data.completion_order < order) {
                        winner = Some((index, data.completion_order));
                    }
                }
                state if state.is_active() => active = true,
                _ => {}
            }
        }
        if let Some((index, _)) = winner {
            for (loser, handle) in handles.iter().copied().enumerate() {
                if loser != index {
                    unsafe { __kome_task_cancel(handle) };
                }
            }
            return index;
        }
        if !active {
            return usize::MAX;
        }
        if let Some(waiter) = current_task() {
            for handle in handles.iter().copied() {
                register_waiter(handle, waiter);
            }
            suspend_current();
            for handle in handles.iter().copied() {
                unregister_waiter(handle, waiter);
            }
        } else if !run_one() {
            park_scheduler(nearest_timer());
        }
    }
}

/// Waits for all tasks, cancelling the remainder after a failure.
///
/// # Safety
///
/// `handles` must point to `length` live task handles.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_all(handles: *const u64, length: usize) -> u8 {
    let handles = unsafe { std::slice::from_raw_parts(handles, length) };
    for (index, handle) in handles.iter().copied().enumerate() {
        let state = unsafe { __kome_task_wait(handle) };
        if state != TASK_COMPLETED {
            for other in handles.iter().copied().skip(index + 1) {
                unsafe { __kome_task_cancel(other) };
            }
            return state;
        }
    }
    TASK_COMPLETED
}

/// Returns a completed task's borrowed raw result slot.
///
/// # Safety
///
/// `handle` must identify a completed live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_result(handle: u64) -> u64 {
    let task = unsafe { record(handle) };
    let data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match data.result {
        Some(TaskResult::Completed(value)) => value,
        _ => 0,
    }
}

/// Returns the error code stored by a failed task.
///
/// # Safety
///
/// `handle` must identify a failed live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_error(handle: u64) -> u64 {
    let task = unsafe { record(handle) };
    let data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match data.result {
        Some(TaskResult::Failed(error)) => error,
        _ => 0,
    }
}

/// Returns whether cancellation was requested or completed.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_is_cancelled(handle: u64) -> u8 {
    u8::from(matches!(
        state(handle),
        TaskState::Cancelled | TaskState::CancellationRequested
    ))
}

/// Returns the unique id assigned to a task handle.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_id(handle: u64) -> TaskId {
    unsafe { record(handle) }.id
}

/// Increments a task handle's owning reference count.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_retain(handle: u64) {
    unsafe { record(handle) }
        .references
        .fetch_add(1, Ordering::Relaxed);
}

/// Releases one task reference and reports whether it was the final owner.
///
/// # Safety
///
/// `handle` must identify a live task and the caller must own one reference.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_release(handle: u64) -> u8 {
    u8::from(
        unsafe { record(handle) }
            .references
            .fetch_sub(1, Ordering::AcqRel)
            == 1,
    )
}

/// Frees a terminal task after its final reference and result are released.
///
/// # Safety
///
/// `handle` must be the final reference reported by [`__kome_task_release`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_dealloc(handle: u64) {
    remove_runnable(handle);
    drop(unsafe { Box::from_raw(handle as *mut TaskRecord) });
}

/// Propagates a non-successful task operation as a runtime diagnostic.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_task_require_completed(state: u8, operation: u8) {
    if state == TASK_COMPLETED {
        return;
    }
    let operation = match operation {
        1 => "all",
        2 => "race",
        3 => "timeout",
        _ => "wait",
    };
    let reason = match state {
        TASK_CANCELLED | TASK_CANCELLATION_REQUESTED => "task was cancelled",
        TASK_FAILED => "task failed",
        _ => "task did not complete",
    };
    let _ = writeln!(std::io::stderr(), "runtime error: {operation}: {reason}");
    std::process::exit(1);
}

/// Validates a successful race winner and returns its index.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_task_require_race_winner(index: usize) -> usize {
    if index != usize::MAX {
        return index;
    }
    let _ = writeln!(
        std::io::stderr(),
        "runtime error: race: every task failed or was cancelled"
    );
    std::process::exit(1);
}
