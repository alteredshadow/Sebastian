use crate::structs::Task;
use serde::Deserialize;
use std::os::unix::fs::PermissionsExt;

#[derive(Deserialize)]
struct ChmodArgs {
    path: String,
    mode: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: ChmodArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mode = match u32::from_str_radix(&args.mode, 8) {
        Ok(m) => m,
        Err(e) => {
            response.set_error(&format!("Invalid mode: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    match std::fs::set_permissions(&args.path, std::fs::Permissions::from_mode(mode)) {
        Ok(_) => {
            response.user_output = format!("Changed permissions of {} to {}", args.path, args.mode);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to chmod: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
