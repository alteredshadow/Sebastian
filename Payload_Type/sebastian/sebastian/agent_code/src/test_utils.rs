//! Test helpers shared across all command and profile tests.
//!
//! Only compiled when running tests (`#[cfg(test)]` in lib.rs).

use crate::structs::{
    AddInternalConnectionMessage, Alert, GetFileFromMythicStruct, InteractiveTaskMessage, Job,
    RemoveInternalConnectionMessage, Response, SendFileToMythicStruct, Task, TaskData,
};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// Build a minimal Task wired to channels so tests can call `execute()` and
/// receive the resulting `Response` without a live Mythic server.
///
/// Returns `(task, response_rx, remove_task_rx)`.
/// - `response_rx` — receive the `Response`(s) sent by the command
/// - `remove_task_rx` — receive the task-id string sent when the command finishes
pub fn make_test_task(
    task_id: &str,
    params: &str,
) -> (Task, mpsc::Receiver<Response>, mpsc::Receiver<String>) {
    // Channels that the command result flows through
    let (send_responses_tx, send_responses_rx) = mpsc::channel::<Response>(32);
    let (remove_running_task_tx, remove_running_task_rx) = mpsc::channel::<String>(32);

    // Channels required by Job but unused in simple command tests
    let (receive_responses_tx, receive_responses_rx) = mpsc::channel::<Value>(32);
    let (send_file_tx, _send_file_rx) = mpsc::channel::<SendFileToMythicStruct>(32);
    let (get_file_tx, _get_file_rx) = mpsc::channel::<GetFileFromMythicStruct>(32);
    let (add_conn_tx, _add_conn_rx) = mpsc::channel::<AddInternalConnectionMessage>(32);
    let (remove_conn_tx, _remove_conn_rx) = mpsc::channel::<RemoveInternalConnectionMessage>(32);
    let (interactive_output_tx, _interactive_output_rx) =
        mpsc::channel::<InteractiveTaskMessage>(32);
    let (alert_tx, _alert_rx) = mpsc::channel::<Alert>(32);

    // The sender for interactive input is dropped here intentionally — commands
    // that read from interactive_task_input_channel (e.g. pty) are not tested
    // via this helper; commands that don't read it see a closed channel, which
    // they never poll.
    let (_interactive_input_tx, interactive_input_rx) = mpsc::channel::<InteractiveTaskMessage>(32);

    let task = Task {
        data: TaskData {
            task_id: task_id.to_string(),
            command: "test".to_string(),
            params: params.to_string(),
            timestamp: 0.0,
        },
        job: Job {
            stop: AtomicBool::new(false),
            receive_responses: receive_responses_rx,
            send_responses: send_responses_tx,
            send_file_to_mythic: send_file_tx,
            get_file_from_mythic: get_file_tx,
            file_transfers: Arc::new(Mutex::new(HashMap::new())),
            save_file_func: |_uuid, _data| {},
            remove_saved_file: |_uuid| {},
            get_saved_file: |_uuid| None,
            add_internal_connection_channel: add_conn_tx,
            remove_internal_connection_channel: remove_conn_tx,
            interactive_task_input_channel: interactive_input_rx,
            interactive_task_output_channel: interactive_output_tx,
            new_alert_channel: alert_tx,
            receive_responses_tx,
        },
        remove_running_task: remove_running_task_tx,
    };

    (task, send_responses_rx, remove_running_task_rx)
}
