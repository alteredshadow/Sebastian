use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct MvArgs {
    source: String,
    destination: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: MvArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    match tokio::fs::rename(&args.source, &args.destination).await {
        Ok(_) => {
            response.user_output = format!("Moved {} -> {}", args.source, args.destination);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to move: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
