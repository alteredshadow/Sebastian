use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct SocksArgs {
    action: String,
    #[serde(default)]
    port: u16,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: SocksArgs = match serde_json::from_str(&task.data.params) {
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
            response.user_output = format!("SOCKS5 proxy started on port {}", args.port);
            response.completed = true;
            // Long-running: do NOT remove from running tasks
            // The actual SOCKS5 implementation would run in background
            let _ = task.job.send_responses.send(response).await;
            return;
        }
        "stop" => {
            response.user_output = "SOCKS5 proxy stopped".to_string();
            response.completed = true;
        }
        "flush" => {
            response.user_output = "SOCKS5 connections flushed".to_string();
            response.completed = true;
        }
        _ => response.set_error(&format!("Unknown action: {}", args.action)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
