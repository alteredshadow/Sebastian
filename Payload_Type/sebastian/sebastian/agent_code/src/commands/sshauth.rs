use crate::structs::Task;
use serde::Deserialize;
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

    let dest = format!("{}@{}", args.username, args.hostname);
    let result = Command::new("ssh")
        .arg("-p").arg(args.port.to_string())
        .arg("-o").arg("StrictHostKeyChecking=no")
        .arg("-o").arg("UserKnownHostsFile=/dev/null")
        .arg("-o").arg("BatchMode=yes")
        .arg(&dest)
        .arg("echo success")
        .output()
        .await;

    match result {
        Ok(output) => {
            if output.status.success() {
                response.user_output = format!("Authentication succeeded for {}", dest);
            } else {
                response.user_output = format!("Authentication failed for {}", dest);
            }
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("SSH auth check failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
