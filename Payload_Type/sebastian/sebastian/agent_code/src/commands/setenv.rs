use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct SetenvArgs {
    name: String,
    value: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: SetenvArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    std::env::set_var(&args.name, &args.value);
    response.user_output = format!("Set {}={}", args.name, args.value);
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
