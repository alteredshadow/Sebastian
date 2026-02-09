use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct CpArgs {
    source: String,
    destination: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: CpArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    match tokio::fs::copy(&args.source, &args.destination).await {
        Ok(bytes) => {
            response.user_output = format!(
                "Copied {} -> {} ({} bytes)",
                args.source, args.destination, bytes
            );
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to copy: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
