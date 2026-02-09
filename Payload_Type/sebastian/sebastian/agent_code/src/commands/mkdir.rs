use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct MkdirArgs {
    path: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: MkdirArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => MkdirArgs { path: task.data.params.clone() },
    };

    match tokio::fs::create_dir_all(&args.path).await {
        Ok(_) => {
            response.user_output = format!("Created directory: {}", args.path);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to create directory: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
