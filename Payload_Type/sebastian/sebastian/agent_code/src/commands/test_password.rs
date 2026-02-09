use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct TestPasswordArgs {
    username: String,
    password: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: TestPasswordArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    #[cfg(target_os = "macos")]
    {
        // Use dscl to test password on macOS
        let result = Command::new("dscl")
            .args(["/Local/Default", "-authonly", &args.username, &args.password])
            .output()
            .await;
        match result {
            Ok(output) => {
                if output.status.success() {
                    response.user_output = format!("Password valid for {}", args.username);
                } else {
                    response.user_output = format!("Password invalid for {}", args.username);
                }
                response.completed = true;
            }
            Err(e) => response.set_error(&format!("Test failed: {}", e)),
        }
    }

    #[cfg(target_os = "linux")]
    {
        // Use su to test password on Linux
        let cmd = format!("echo '{}' | su -c 'echo success' {} 2>&1", args.password.replace('\'', "'\\''"), args.username);
        match Command::new("/bin/sh").arg("-c").arg(&cmd).output().await {
            Ok(output) => {
                let out = String::from_utf8_lossy(&output.stdout);
                if out.contains("success") {
                    response.user_output = format!("Password valid for {}", args.username);
                } else {
                    response.user_output = format!("Password invalid for {}", args.username);
                }
                response.completed = true;
            }
            Err(e) => response.set_error(&format!("Test failed: {}", e)),
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
