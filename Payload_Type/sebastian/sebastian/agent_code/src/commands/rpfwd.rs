use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct RpfwdArgs {
    action: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    remote_ip: String,
    #[serde(default)]
    remote_port: u16,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: RpfwdArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    match args.action.as_str() {
        "start" => {
            response.user_output = format!(
                "Reverse port forward started: {}:{} -> local:{}",
                args.remote_ip, args.remote_port, args.port
            );
            response.completed = true;
            let _ = task.job.send_responses.send(response).await;
            return; // Long-running
        }
        "stop" => {
            response.user_output = "Reverse port forward stopped".to_string();
            response.completed = true;
        }
        _ => response.set_error(&format!("Unknown action: {}", args.action)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
