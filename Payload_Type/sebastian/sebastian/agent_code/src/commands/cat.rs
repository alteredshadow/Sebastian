use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct CatArgs {
    path: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: CatArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => CatArgs { path: task.data.params.clone() },
    };

    match tokio::fs::read_to_string(&args.path).await {
        Ok(contents) => {
            response.user_output = contents;
            response.completed = true;
        }
        Err(e) => {
            response.set_error(&format!("Failed to read file: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
