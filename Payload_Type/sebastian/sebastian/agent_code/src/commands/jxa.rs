use crate::structs::Task;
use base64::Engine;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct JxaArgs {
    code: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: JxaArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Decode base64 encoded JXA code
    let decoded = match base64::engine::general_purpose::STANDARD.decode(&args.code) {
        Ok(d) => d,
        Err(e) => {
            response.set_error(&format!("Failed to decode base64: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let code_str = match String::from_utf8(decoded) {
        Ok(s) => s,
        Err(e) => {
            response.set_error(&format!("Invalid UTF-8 in code: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Execute JXA via osascript
    match Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &code_str])
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            if !stderr.is_empty() && stdout.is_empty() {
                response.user_output = stderr.to_string();
            } else {
                response.user_output = stdout.to_string();
            }
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to execute JXA: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
