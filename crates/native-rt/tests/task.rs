use kome_native_rt::task::{
    __kome_task_all, __kome_task_cancel, __kome_task_complete, __kome_task_create,
    __kome_task_dealloc, __kome_task_error, __kome_task_fail, __kome_task_id,
    __kome_task_is_cancelled, __kome_task_race, __kome_task_release, __kome_task_require_completed,
    __kome_task_result, __kome_task_start, __kome_task_state, __kome_task_wait,
    __kome_task_wait_timeout, TASK_CANCELLED, TASK_COMPLETED, TASK_FAILED,
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

#[test]
fn running_tasks_record_a_cooperative_cancellation_request() {
    let task = __kome_task_create();
    unsafe {
        __kome_task_start(task);
        __kome_task_cancel(task);
        __kome_task_cancel(task);
        assert_eq!(__kome_task_is_cancelled(task), 1);
        __kome_task_complete(task, 42);
        assert_eq!(__kome_task_wait(task), TASK_COMPLETED);
        assert_eq!(__kome_task_result(task), 42);
        assert_eq!(__kome_task_release(task), 1);
        __kome_task_dealloc(task);
    }
}

#[test]
fn all_propagates_failure_and_cancels_remaining_tasks() {
    let failed = __kome_task_create();
    let remaining = __kome_task_create();
    unsafe {
        __kome_task_fail(failed, 9);
        let handles = [failed, remaining];
        assert_eq!(
            __kome_task_all(handles.as_ptr(), handles.len()),
            TASK_FAILED
        );
        assert_eq!(__kome_task_state(remaining), TASK_CANCELLED);
        assert_eq!(__kome_task_release(failed), 1);
        assert_eq!(__kome_task_release(remaining), 1);
        __kome_task_dealloc(failed);
        __kome_task_dealloc(remaining);
    }
}

#[test]
fn race_ignores_failure_and_selects_later_success() {
    let failed = __kome_task_create();
    let success = __kome_task_create();
    unsafe { __kome_task_fail(failed, 1) };
    let worker = std::thread::spawn(move || unsafe {
        std::thread::sleep(std::time::Duration::from_millis(5));
        __kome_task_complete(success, 42);
    });
    let handles = [failed, success];
    assert_eq!(
        unsafe { __kome_task_race(handles.as_ptr(), handles.len()) },
        1
    );
    worker.join().unwrap();
    unsafe {
        assert_eq!(__kome_task_result(success), 42);
        assert_eq!(__kome_task_release(failed), 1);
        assert_eq!(__kome_task_release(success), 1);
        __kome_task_dealloc(failed);
        __kome_task_dealloc(success);
    }
}

#[test]
fn timeout_cancels_a_pending_task_without_busy_waiting() {
    let task = __kome_task_create();
    unsafe {
        assert_eq!(__kome_task_wait_timeout(task, 1), TASK_CANCELLED);
        assert_eq!(__kome_task_state(task), TASK_CANCELLED);
        assert_eq!(__kome_task_release(task), 1);
        __kome_task_dealloc(task);
    }
}

#[test]
fn terminal_states_do_not_transition_backwards() {
    let task = __kome_task_create();
    unsafe {
        __kome_task_cancel(task);
        __kome_task_start(task);
        __kome_task_complete(task, 42);
        __kome_task_fail(task, 9);
        assert_eq!(__kome_task_state(task), TASK_CANCELLED);
        assert_eq!(__kome_task_release(task), 1);
        __kome_task_dealloc(task);
    }
}

#[test]
fn race_uses_completion_order_instead_of_argument_order() {
    let first = __kome_task_create();
    let second = __kome_task_create();
    unsafe {
        __kome_task_complete(second, 2);
        __kome_task_complete(first, 1);
        let handles = [first, second];
        assert_eq!(__kome_task_race(handles.as_ptr(), handles.len()), 1);
        assert_eq!(__kome_task_release(first), 1);
        assert_eq!(__kome_task_release(second), 1);
        __kome_task_dealloc(first);
        __kome_task_dealloc(second);
    }
}

#[test]
fn cancelled_wait_error_child() {
    if std::env::var_os("KOME_TASK_ERROR_CHILD").is_some() {
        __kome_task_require_completed(TASK_CANCELLED, 0);
    }
}

#[test]
fn cancelled_wait_reports_a_runtime_error_without_panicking() {
    let output = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "cancelled_wait_error_child", "--nocapture"])
        .env("KOME_TASK_ERROR_CHILD", "1")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("wait: task was cancelled"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
