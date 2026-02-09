use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct CaffeinateArgs {
    #[serde(default)]
    enable: bool,
}

// IOKit FFI for power management assertions
#[link(name = "IOKit", kind = "framework")]
extern "C" {
    fn IOPMAssertionCreateWithName(
        assertion_type: *const std::ffi::c_void,
        assertion_level: u32,
        reason_for_activity: *const std::ffi::c_void,
        assertion_id: *mut u32,
    ) -> i32;
    fn IOPMAssertionRelease(assertion_id: u32) -> i32;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFStringCreateWithCString(
        alloc: *const std::ffi::c_void,
        c_str: *const i8,
        encoding: u32,
    ) -> *const std::ffi::c_void;
}

const K_CFSTRING_ENCODING_UTF8: u32 = 0x08000100;
const K_IOPM_ASSERTION_LEVEL_ON: u32 = 255;

use std::sync::atomic::{AtomicU32, Ordering};
static ASSERTION_ID: AtomicU32 = AtomicU32::new(0);
static CAFFEINATED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: CaffeinateArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    if args.enable {
        if CAFFEINATED.load(Ordering::Relaxed) {
            response.user_output = "Already caffeinated".to_string();
            response.completed = true;
        } else {
            unsafe {
                let assertion_type = CFStringCreateWithCString(
                    std::ptr::null(),
                    b"PreventUserIdleSystemSleep\0".as_ptr() as *const i8,
                    K_CFSTRING_ENCODING_UTF8,
                );
                let reason = CFStringCreateWithCString(
                    std::ptr::null(),
                    b"User requested caffeinate\0".as_ptr() as *const i8,
                    K_CFSTRING_ENCODING_UTF8,
                );

                let mut assertion_id: u32 = 0;
                let result = IOPMAssertionCreateWithName(
                    assertion_type,
                    K_IOPM_ASSERTION_LEVEL_ON,
                    reason,
                    &mut assertion_id,
                );

                if result == 0 {
                    ASSERTION_ID.store(assertion_id, Ordering::Relaxed);
                    CAFFEINATED.store(true, Ordering::Relaxed);
                    response.user_output = "Caffeinate enabled - system will not sleep".to_string();
                    response.completed = true;
                } else {
                    response.set_error(&format!(
                        "Failed to create power assertion: error {}",
                        result
                    ));
                }
            }
        }
    } else {
        if !CAFFEINATED.load(Ordering::Relaxed) {
            response.user_output = "Not currently caffeinated".to_string();
            response.completed = true;
        } else {
            let assertion_id = ASSERTION_ID.load(Ordering::Relaxed);
            let result = unsafe { IOPMAssertionRelease(assertion_id) };
            if result == 0 {
                CAFFEINATED.store(false, Ordering::Relaxed);
                response.user_output = "Caffeinate disabled - system can sleep normally".to_string();
                response.completed = true;
            } else {
                response.set_error(&format!(
                    "Failed to release power assertion: error {}",
                    result
                ));
            }
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
