use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct SshArgs {
    hostname: String,
    #[serde(default = "default_port")]
    port: u16,
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    private_key: String,
    command: String,
}

fn default_port() -> u16 { 22 }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: SshArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let dest = format!("{}@{}", args.username, args.hostname);
    let mut cmd = Command::new("ssh");
    cmd.arg("-p").arg(args.port.to_string())
        .arg("-o").arg("StrictHostKeyChecking=no")
        .arg("-o").arg("UserKnownHostsFile=/dev/null")
        .arg(&dest)
        .arg(&args.command);

    match cmd.output().await {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("SSH failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
