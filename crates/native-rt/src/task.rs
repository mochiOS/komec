//! Reference-counted task state and the stable ABI used by generated code.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex};

static NEXT_TASK_ID: AtomicUsize = AtomicUsize::new(1);

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

/// Observable lifecycle state of a runtime task.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u8)]
pub enum TaskState {
    Pending = TASK_PENDING,
    Running = TASK_RUNNING,
    Completed = TASK_COMPLETED,
    Cancelled = TASK_CANCELLED,
    Failed = TASK_FAILED,
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
    task.ready.notify_all();
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
    if !matches!(data.state, TaskState::Completed | TaskState::Failed) {
        data.state = TaskState::Cancelled;
        data.result = Some(TaskResult::Cancelled);
        task.ready.notify_all();
    }
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
    while matches!(data.state, TaskState::Pending | TaskState::Running) {
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
