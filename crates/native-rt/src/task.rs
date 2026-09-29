//! Reference-counted task state and the stable ABI used by generated code.
//!
//! The first scheduler is single-threaded and run-to-completion: generated
//! code marks a task running, computes its result, and completes it through
//! this ABI. Waiting is expressed as a condition-variable boundary so a later
//! cooperative executor can suspend and wake waiters without changing codegen.

use std::io::Write;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::time::Duration;

static NEXT_TASK_ID: AtomicUsize = AtomicUsize::new(1);
static NEXT_COMPLETION: AtomicU64 = AtomicU64::new(1);
static SCHEDULER_WAKE: OnceLock<(Mutex<u64>, Condvar)> = OnceLock::new();

/// Stable identifier assigned to one runtime task.
pub type TaskId = u64;

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
}

/// Result stored by a task after it leaves the pending/running states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TaskResult {
    Completed(u64),
    Cancelled,
    Failed(u64),
}

#[derive(Debug)]
struct TaskData {
    state: TaskState,
    result: Option<TaskResult>,
    completion_order: u64,
}

#[derive(Debug)]
struct TaskRecord {
    id: TaskId,
    references: AtomicUsize,
    data: Mutex<TaskData>,
    ready: Condvar,
}

unsafe fn record(handle: u64) -> &'static TaskRecord {
    unsafe { &*(handle as *const TaskRecord) }
}

/// Allocates a pending task with one owning reference.
#[unsafe(no_mangle)]
pub extern "C" fn __kome_task_create() -> u64 {
    Box::into_raw(Box::new(TaskRecord {
        id: NEXT_TASK_ID.fetch_add(1, Ordering::Relaxed) as TaskId,
        references: AtomicUsize::new(1),
        data: Mutex::new(TaskData {
            state: TaskState::Pending,
            result: None,
            completion_order: 0,
        }),
        ready: Condvar::new(),
    })) as u64
}

/// Transitions a pending task to the running state.
///
/// # Safety
///
/// `handle` must identify a live task allocated by [`__kome_task_create`].
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

/// Stores a task result and wakes every waiter.
///
/// # Safety
///
/// `handle` must identify a live task and `result` must be the raw value whose
/// concrete type is tracked by generated code.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_complete(handle: u64, result: u64) {
    let task = unsafe { record(handle) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    data.result = Some(TaskResult::Completed(result));
    data.state = TaskState::Completed;
    data.completion_order = NEXT_COMPLETION.fetch_add(1, Ordering::Relaxed);
    task.ready.notify_all();
    wake_scheduler();
}

/// Stores a runtime error code and wakes every waiter.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_fail(handle: u64, error: u64) {
    let task = unsafe { record(handle) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    data.result = Some(TaskResult::Failed(error));
    data.state = TaskState::Failed;
    task.ready.notify_all();
    wake_scheduler();
}

/// Cancels a task that has not completed and wakes every waiter.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_cancel(handle: u64) {
    let task = unsafe { record(handle) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    match data.state {
        TaskState::Pending => {
            data.state = TaskState::Cancelled;
            data.result = Some(TaskResult::Cancelled);
            task.ready.notify_all();
            wake_scheduler();
        }
        TaskState::Running => data.state = TaskState::CancellationRequested,
        TaskState::CancellationRequested
        | TaskState::Completed
        | TaskState::Failed
        | TaskState::Cancelled => {}
    }
}

fn wake_scheduler() {
    let (generation, ready) = SCHEDULER_WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()));
    let mut generation = generation
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    *generation = generation.wrapping_add(1);
    ready.notify_all();
}

/// Returns a task's current lifecycle state without waiting.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_state(handle: u64) -> u8 {
    let task = unsafe { record(handle) };
    task.data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .state as u8
}

/// Waits up to `milliseconds` for a task and returns its resulting state.
///
/// A deadline expiration requests cancellation before returning.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_wait_timeout(handle: u64, milliseconds: u64) -> u8 {
    let task = unsafe { record(handle) };
    let data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let (data, timed_out) = task
        .ready
        .wait_timeout_while(data, Duration::from_millis(milliseconds), |data| {
            matches!(
                data.state,
                TaskState::Pending | TaskState::Running | TaskState::CancellationRequested
            )
        })
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    let state = data.state;
    drop(data);
    if timed_out.timed_out()
        && matches!(
            state,
            TaskState::Pending | TaskState::Running | TaskState::CancellationRequested
        )
    {
        unsafe { __kome_task_cancel(handle) };
        return unsafe { __kome_task_state(handle) };
    }
    state as u8
}

/// Waits for and returns the index of the first successfully completed task.
///
/// `usize::MAX` is returned when every task is failed or cancelled.
///
/// # Safety
///
/// `handles` must point to `length` live task handles for the duration of the
/// call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_race(handles: *const u64, length: usize) -> usize {
    let handles = unsafe { std::slice::from_raw_parts(handles, length) };
    loop {
        let (generation, ready) = SCHEDULER_WAKE.get_or_init(|| (Mutex::new(0), Condvar::new()));
        let observed = *generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
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
                TaskState::Pending | TaskState::Running | TaskState::CancellationRequested => {
                    active = true;
                }
                TaskState::Cancelled | TaskState::Failed => {}
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
        let mut current = generation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        while *current == observed {
            current = ready
                .wait(current)
                .unwrap_or_else(|poisoned| poisoned.into_inner());
        }
    }
}

/// Waits for all tasks, cancelling the remainder after the first failure.
///
/// The returned value is [`TASK_COMPLETED`] on success or the terminal state
/// that prevented the aggregate from completing.
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

/// Propagates a non-successful task operation as a runtime error.
///
/// `operation` is `0` for wait, `1` for all, `2` for race, and `3` for
/// timeout. Successful states return normally; errors terminate through the
/// same diagnostic path used by other unrecoverable Kome runtime errors.
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

/// Validates that a race produced a successful winner and returns its index.
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

/// Returns whether cancellation was requested or completed for a task.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_is_cancelled(handle: u64) -> u8 {
    let task = unsafe { record(handle) };
    let data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    u8::from(matches!(
        data.state,
        TaskState::Cancelled | TaskState::CancellationRequested
    ))
}

/// Blocks until the task reaches a terminal state and returns that state.
///
/// This ABI boundary deliberately hides the blocking mechanism so the runtime
/// can later replace it with cooperative suspension and waiter wakeups.
///
/// # Safety
///
/// `handle` must identify a live task.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_wait(handle: u64) -> u8 {
    let task = unsafe { record(handle) };
    let mut data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    while matches!(
        data.state,
        TaskState::Pending | TaskState::Running | TaskState::CancellationRequested
    ) {
        data = task
            .ready
            .wait(data)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
    data.state as u8
}

/// Returns the completed task's raw result.
///
/// # Safety
///
/// `handle` must identify a live task in the completed state. The result is
/// borrowed from the task; generated code must retain managed values it keeps.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_result(handle: u64) -> u64 {
    let task = unsafe { record(handle) };
    let data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    debug_assert_eq!(data.state, TaskState::Completed);
    match data.result {
        Some(TaskResult::Completed(value)) => value,
        _ => 0,
    }
}

/// Returns the error code stored by a failed task.
///
/// # Safety
///
/// `handle` must identify a live task in the failed state.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_error(handle: u64) -> u64 {
    let task = unsafe { record(handle) };
    let data = task
        .data
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    debug_assert_eq!(data.state, TaskState::Failed);
    match data.result {
        Some(TaskResult::Failed(error)) => error,
        _ => 0,
    }
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

/// Releases a task reference and returns `1` to its final owner.
///
/// # Safety
///
/// `handle` must identify a live task and the caller must own one reference.
/// When this returns `1`, generated code must release an unconsumed managed
/// result and call [`__kome_task_dealloc`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_release(handle: u64) -> u8 {
    u8::from(
        unsafe { record(handle) }
            .references
            .fetch_sub(1, Ordering::AcqRel)
            == 1,
    )
}

/// Frees the final task reference after its managed result has been released.
///
/// # Safety
///
/// `handle` must be the final reference identified by [`__kome_task_release`].
#[unsafe(no_mangle)]
pub unsafe extern "C" fn __kome_task_dealloc(handle: u64) {
    drop(unsafe { Box::from_raw(handle as *mut TaskRecord) });
}
