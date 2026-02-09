use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct HeadArgs {
    path: String,
    #[serde(default = "default_lines")]
    lines: usize,
}

fn default_lines() -> usize { 10 }

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: HeadArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    match tokio::fs::read_to_string(&args.path).await {
        Ok(contents) => {
            let result: String = contents.lines().take(args.lines).collect::<Vec<_>>().join("\n");
            response.user_output = result;
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to read file: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
