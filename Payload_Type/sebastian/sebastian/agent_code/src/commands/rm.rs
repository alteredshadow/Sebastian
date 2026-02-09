use crate::structs::{RmFiles, Task};
use crate::utils;
use serde::Deserialize;

#[derive(Deserialize)]
struct RmArgs {
    path: String,
    #[serde(default)]
    recursive: bool,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: RmArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => RmArgs { path: task.data.params.clone(), recursive: false },
    };

    let metadata = match tokio::fs::metadata(&args.path).await {
        Ok(m) => m,
        Err(e) => {
            response.set_error(&format!("Path not found: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let result = if metadata.is_dir() {
        if args.recursive {
            tokio::fs::remove_dir_all(&args.path).await
        } else {
            tokio::fs::remove_dir(&args.path).await
        }
    } else {
        tokio::fs::remove_file(&args.path).await
    };

    match result {
        Ok(_) => {
            response.user_output = format!("Removed: {}", args.path);
            response.completed = true;
            response.removed_files = Some(vec![RmFiles {
                path: args.path,
                host: utils::get_hostname(),
            }]);
        }
        Err(e) => response.set_error(&format!("Failed to remove: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
