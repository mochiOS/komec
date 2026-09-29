use kome_native_rt::task::{
    __kome_task_cancel, __kome_task_complete, __kome_task_create, __kome_task_dealloc,
    __kome_task_error, __kome_task_fail, __kome_task_id, __kome_task_release, __kome_task_result,
    __kome_task_start, __kome_task_wait, TASK_CANCELLED, TASK_COMPLETED, TASK_FAILED,
};

#[test]
fn creates_runs_completes_and_reads_a_task() {
    let task = __kome_task_create();
    unsafe {
        __kome_task_start(task);
        __kome_task_complete(task, 42);
        assert_eq!(__kome_task_wait(task), TASK_COMPLETED);
        assert_eq!(__kome_task_result(task), 42);
        assert_eq!(__kome_task_release(task), 1);
        __kome_task_dealloc(task);
    }
}

#[test]
fn assigns_unique_task_ids() {
    let first = __kome_task_create();
    let second = __kome_task_create();
    unsafe {
        assert_ne!(__kome_task_id(first), __kome_task_id(second));
        __kome_task_complete(first, 0);
        __kome_task_complete(second, 0);
        assert_eq!(__kome_task_release(first), 1);
        assert_eq!(__kome_task_release(second), 1);
        __kome_task_dealloc(first);
        __kome_task_dealloc(second);
    }
}

#[test]
fn preserves_failed_state_and_error() {
    let task = __kome_task_create();
    unsafe {
        __kome_task_start(task);
        __kome_task_fail(task, 7);
        assert_eq!(__kome_task_wait(task), TASK_FAILED);
        assert_eq!(__kome_task_error(task), 7);
        assert_eq!(__kome_task_release(task), 1);
        __kome_task_dealloc(task);
    }
}

#[test]
fn wakes_waiters_when_cancelled() {
    let task = __kome_task_create();
    unsafe {
        __kome_task_cancel(task);
        assert_eq!(__kome_task_wait(task), TASK_CANCELLED);
        assert_eq!(__kome_task_release(task), 1);
        __kome_task_dealloc(task);
    }
}
