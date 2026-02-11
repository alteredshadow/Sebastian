use crate::structs::Task;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

#[derive(Deserialize)]
struct SshAuthArgs {
    hostname: String,
    #[serde(default = "default_port")]
    port: u16,
    username: String,
    #[serde(default)]
    password: String,
}

fn default_port() -> u16 { 22 }

#[derive(Serialize)]
struct SshResult {
    host: String,
    username: String,
    secret: String,
    status: String,
    output: String,
    copy_status: String,
    success: bool,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: SshAuthArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let dest = format!("{}:{}", args.hostname, args.port);
    let result = Command::new("ssh")
        .arg("-p").arg(args.port.to_string())
        .arg("-o").arg("StrictHostKeyChecking=no")
        .arg("-o").arg("UserKnownHostsFile=/dev/null")
        .arg("-o").arg("BatchMode=yes")
        .arg(format!("{}@{}", args.username, args.hostname))
        .arg("echo success")
        .output()
        .await;

    let ssh_result = match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let success = output.status.success();
            SshResult {
                host: dest,
                username: args.username,
                secret: args.password,
                status: if success { "success".to_string() } else { "failed".to_string() },
                output: if success { stdout } else { stderr },
                copy_status: String::new(),
                success,
            }
        }
        Err(e) => SshResult {
            host: dest,
            username: args.username,
            secret: args.password,
            status: "error".to_string(),
            output: e.to_string(),
            copy_status: String::new(),
            success: false,
        },
    };

    // Browser script expects JSON array of SshResult objects
    let results = vec![ssh_result];
    response.user_output = serde_json::to_string_pretty(&results)
        .unwrap_or_else(|_| "[]".to_string());
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
