use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct XpcArgs {
    #[serde(default)]
    command: String,
    #[serde(default)]
    servicename: String,
    #[serde(default)]
    program: String,
    #[serde(default)]
    file: String,
    #[serde(default)]
    pid: i32,
    #[serde(default)]
    data: String,
    #[serde(default)]
    uid: i32,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: XpcArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Determine which XPC subcommand based on the task command or args.command
    let xpc_command = if !args.command.is_empty() {
        args.command.clone()
    } else {
        task.data.command.clone()
    };

    let result = match xpc_command.as_str() {
        "xpc_service" | "list" => {
            if args.servicename.is_empty() {
                Command::new("launchctl")
                    .args(["list"])
                    .output()
                    .await
            } else {
                Command::new("launchctl")
                    .args(["list", &args.servicename])
                    .output()
                    .await
            }
        }
        "xpc_start" | "start" => {
            Command::new("launchctl")
                .args(["start", &args.servicename])
                .output()
                .await
        }
        "xpc_stop" | "stop" => {
            Command::new("launchctl")
                .args(["stop", &args.servicename])
                .output()
                .await
        }
        "xpc_status" | "print" => {
            let target = if args.pid > 0 {
                format!("{}", args.pid)
            } else if args.uid > 0 {
                format!("user/{}", args.uid)
            } else {
                format!("system/{}", args.servicename)
            };
            Command::new("launchctl")
                .args(["print", &target])
                .output()
                .await
        }
        "xpc_submit" | "load" => {
            if args.file.is_empty() {
                Command::new("launchctl")
                    .args(["load", &args.servicename])
                    .output()
                    .await
            } else {
                Command::new("launchctl")
                    .args(["load", &args.file])
                    .output()
                    .await
            }
        }
        "xpc_remove" | "unload" => {
            if args.file.is_empty() {
                Command::new("launchctl")
                    .args(["remove", &args.servicename])
                    .output()
                    .await
            } else {
                Command::new("launchctl")
                    .args(["unload", &args.file])
                    .output()
                    .await
            }
        }
        "enable" => {
            let target = format!("system/{}", args.servicename);
            Command::new("launchctl")
                .args(["enable", &target])
                .output()
                .await
        }
        "disable" => {
            let target = format!("system/{}", args.servicename);
            Command::new("launchctl")
                .args(["disable", &target])
                .output()
                .await
        }
        _ => {
            response.set_error(&format!("Unknown XPC command: {}", xpc_command));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("XPC command failed: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
