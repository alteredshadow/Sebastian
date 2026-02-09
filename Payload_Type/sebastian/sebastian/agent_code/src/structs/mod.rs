use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use tokio::sync::mpsc;

// ============================================================================
// Interactive Task Message Types
// ============================================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[repr(i32)]
pub enum InteractiveTaskType {
    Input = 0,
    Output = 1,
    Error = 2,
    Exit = 3,
    Escape = 4,
    CtrlA = 5,
    CtrlB = 6,
    CtrlC = 7,
    CtrlD = 8,
    CtrlE = 9,
    CtrlF = 10,
    CtrlG = 11,
    Backspace = 12,
    Tab = 13,
    CtrlK = 14,
    CtrlL = 15,
    CtrlN = 16,
    CtrlP = 17,
    CtrlQ = 18,
    CtrlR = 19,
    CtrlS = 20,
    CtrlU = 21,
    CtrlW = 22,
    CtrlY = 23,
    CtrlZ = 24,
}

impl InteractiveTaskType {
    pub fn is_valid(value: i32) -> bool {
        value >= 0 && value <= 24
    }
}

// ============================================================================
// Alert Levels
// ============================================================================

pub const ALERT_LEVEL_WARNING: &str = "warning";
pub const ALERT_LEVEL_INFO: &str = "info";
pub const ALERT_LEVEL_DEBUG: &str = "debug";

// ============================================================================
// CheckIn Messages
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct CheckInMessage {
    pub action: String,
    pub ips: Vec<String>,
    pub os: String,
    pub user: String,
    pub host: String,
    pub pid: i32,
    pub uuid: String,
    pub architecture: String,
    pub domain: String,
    pub integrity_level: i32,
    pub external_ip: String,
    pub process_name: String,
    pub sleep_info: String,
    pub cwd: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct CheckInMessageResponse {
    pub action: Option<String>,
    pub id: Option<String>,
    pub status: Option<String>,
}

// ============================================================================
// EKE Key Exchange
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct EkeKeyExchangeMessage {
    pub action: String,
    pub pub_key: String,
    pub session_id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct EkeKeyExchangeMessageResponse {
    pub action: Option<String>,
    pub uuid: Option<String>,
    pub session_id: Option<String>,
    pub session_key: Option<String>,
}

// ============================================================================
// Mythic Messages (main communication)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct MythicMessage {
    pub action: String,
    pub tasking_size: i32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delegates: Option<Vec<DelegateMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub responses: Option<Vec<Response>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub socks: Option<Vec<SocksMsg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rpfwds: Option<Vec<SocksMsg>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub edges: Option<Vec<P2PConnectionMessage>>,
    #[serde(rename = "interactive", skip_serializing_if = "Option::is_none")]
    pub interactive_tasks: Option<Vec<InteractiveTaskMessage>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alerts: Option<Vec<Alert>>,
}

impl MythicMessage {
    pub fn new_get_tasking() -> Self {
        Self {
            action: "get_tasking".to_string(),
            tasking_size: -1,
            delegates: None,
            responses: None,
            socks: None,
            rpfwds: None,
            edges: None,
            interactive_tasks: None,
            alerts: None,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct MythicMessageResponse {
    pub action: Option<String>,
    #[serde(default)]
    pub tasks: Vec<TaskData>,
    #[serde(default)]
    pub delegates: Vec<DelegateMessage>,
    #[serde(default)]
    pub socks: Vec<SocksMsg>,
    #[serde(default, rename = "rpfwd")]
    pub rpfwds: Vec<SocksMsg>,
    #[serde(default)]
    pub responses: Vec<HashMap<String, Value>>,
    #[serde(default, rename = "interactive")]
    pub interactive_tasks: Vec<InteractiveTaskMessage>,
}

// ============================================================================
// Task
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskData {
    #[serde(rename = "id")]
    pub task_id: String,
    pub command: String,
    #[serde(rename = "parameters")]
    pub params: String,
    #[serde(default)]
    pub timestamp: f64,
}

/// A running task with its associated Job channels
pub struct Task {
    pub data: TaskData,
    pub job: Job,
    pub remove_running_task: mpsc::Sender<String>,
}

impl Task {
    pub fn new_response(&self) -> Response {
        Response {
            task_id: self.data.task_id.clone(),
            ..Response::default()
        }
    }

    pub fn should_stop(&self) -> bool {
        self.job.stop.load(std::sync::atomic::Ordering::Relaxed)
    }
}

// ============================================================================
// TaskStub (for listing running tasks)
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct TaskStub {
    pub command: String,
    pub params: String,
    pub id: String,
}

impl From<&Task> for TaskStub {
    fn from(task: &Task) -> Self {
        Self {
            command: task.data.command.clone(),
            params: task.data.params.clone(),
            id: task.data.task_id.clone(),
        }
    }
}

// ============================================================================
// Job (execution context for a task)
// ============================================================================

pub struct Job {
    pub stop: std::sync::atomic::AtomicBool,
    pub receive_responses: mpsc::Receiver<Value>,
    pub send_responses: mpsc::Sender<Response>,
    pub send_file_to_mythic: mpsc::Sender<SendFileToMythicStruct>,
    pub get_file_from_mythic: mpsc::Sender<GetFileFromMythicStruct>,
    pub file_transfers: std::sync::Mutex<HashMap<String, mpsc::Sender<Value>>>,
    pub save_file_func: fn(file_uuid: &str, data: &[u8]),
    pub remove_saved_file: fn(file_uuid: &str),
    pub get_saved_file: fn(file_uuid: &str) -> Option<Vec<u8>>,
    pub add_internal_connection_channel: mpsc::Sender<AddInternalConnectionMessage>,
    pub remove_internal_connection_channel: mpsc::Sender<RemoveInternalConnectionMessage>,
    pub interactive_task_input_channel: mpsc::Receiver<InteractiveTaskMessage>,
    pub interactive_task_output_channel: mpsc::Sender<InteractiveTaskMessage>,
    pub new_alert_channel: mpsc::Sender<Alert>,
    /// Sender side of receive_responses - used for routing responses back to this job
    pub receive_responses_tx: mpsc::Sender<Value>,
}

// ============================================================================
// Response
// ============================================================================

#[derive(Debug, Clone, Default, Serialize)]
pub struct Response {
    pub task_id: String,
    pub user_output: String,
    pub completed: bool,
    pub status: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub file_browser: Option<FileBrowser>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub removed_files: Option<Vec<RmFiles>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub processes: Option<Vec<ProcessDetails>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_uuid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub upload: Option<FileUploadMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub download: Option<FileDownloadMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub keylogs: Option<Vec<Keylog>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub artifacts: Option<Vec<Artifact>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub alerts: Option<Vec<Alert>>,
    #[serde(rename = "callback", skip_serializing_if = "Option::is_none")]
    pub callback_update: Option<CallbackUpdate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub process_response: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stdout: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub stderr: Option<String>,
}

impl Response {
    pub fn set_error(&mut self, err_string: &str) {
        self.user_output = err_string.to_string();
        self.status = "error".to_string();
        self.completed = true;
    }
}

// ============================================================================
// File Browser
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct FileBrowser {
    pub files: Vec<FileData>,
    pub is_file: bool,
    pub permissions: FilePermission,
    #[serde(rename = "name")]
    pub filename: String,
    pub parent_path: String,
    pub success: bool,
    #[serde(rename = "size")]
    pub file_size: i64,
    #[serde(rename = "modify_time")]
    pub last_modified: i64,
    #[serde(rename = "access_time")]
    pub last_access: i64,
    pub update_deleted: bool,
    pub set_as_user_output: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileData {
    pub is_file: bool,
    pub permissions: FilePermission,
    pub name: String,
    pub full_name: String,
    #[serde(rename = "size")]
    pub file_size: i64,
    #[serde(rename = "modify_time")]
    pub last_modified: i64,
    #[serde(rename = "access_time")]
    pub last_access: i64,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct FilePermission {
    pub uid: i32,
    pub gid: i32,
    pub permissions: String,
    pub setuid: bool,
    pub setgid: bool,
    pub sticky: bool,
    pub user: String,
    pub group: String,
    pub symlink: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileBrowserArguments {
    pub file: Option<String>,
    pub path: Option<String>,
    pub host: Option<String>,
    pub file_browser: Option<bool>,
    pub depth: Option<i32>,
}

// ============================================================================
// Process
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct ProcessDetails {
    pub process_id: i32,
    pub parent_process_id: i32,
    #[serde(rename = "architecture")]
    pub arch: String,
    pub user: String,
    pub bin_path: String,
    #[serde(rename = "args")]
    pub arguments: Vec<String>,
    #[serde(rename = "env")]
    pub environment: HashMap<String, String>,
    #[serde(rename = "sandboxpath")]
    pub sandbox_path: String,
    pub scripting_properties: HashMap<String, Value>,
    pub name: String,
    #[serde(rename = "bundleid")]
    pub bundle_id: String,
    pub update_deleted: bool,
    pub additional_information: HashMap<String, Value>,
}

// ============================================================================
// Keylog & Artifacts
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct Keylog {
    pub user: String,
    pub window_title: String,
    pub keystrokes: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct Artifact {
    pub base_artifact: String,
    #[serde(rename = "Artifact")]
    pub artifact: String,
}

// ============================================================================
// Alerts
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct Alert {
    pub alert: String,
    pub send_webhook: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub level: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook_alert: Option<HashMap<String, Value>>,
}

// ============================================================================
// Callback Update
// ============================================================================

#[derive(Debug, Clone, Default, Serialize)]
pub struct CallbackUpdate {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub impersonation_context: Option<String>,
}

// ============================================================================
// File Transfer Messages
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct FileUploadMessage {
    pub chunk_size: i32,
    pub total_chunks: i32,
    pub file_id: String,
    pub chunk_num: i32,
    pub full_path: String,
    pub chunk_data: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct FileDownloadMessage {
    pub total_chunks: i32,
    pub chunk_num: i32,
    pub full_path: String,
    pub filename: String,
    pub chunk_data: String,
    pub file_id: String,
    pub is_screenshot: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct FileUploadMessageResponse {
    pub total_chunks: Option<i32>,
    pub chunk_num: Option<i32>,
    pub chunk_data: Option<String>,
    pub file_id: Option<String>,
}

pub struct SendFileToMythicStruct {
    pub task_id: String,
    pub is_screenshot: bool,
    pub file_name: String,
    pub send_user_status_updates: bool,
    pub full_path: String,
    pub data: Option<Vec<u8>>,
    pub finished_transfer: mpsc::Sender<i32>,
    pub tracking_uuid: String,
    pub file_transfer_response: Option<mpsc::Sender<Value>>,
}

pub struct GetFileFromMythicStruct {
    pub task_id: String,
    pub full_path: String,
    pub file_id: String,
    pub send_user_status_updates: bool,
    pub received_chunk_channel: mpsc::Sender<Vec<u8>>,
    pub tracking_uuid: String,
    pub file_transfer_response: Option<mpsc::Sender<Value>>,
}

// ============================================================================
// Delegate / P2P Messages
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DelegateMessage {
    pub message: String,
    pub uuid: String,
    pub c2_profile: String,
    #[serde(rename = "new_uuid", default)]
    pub mythic_uuid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct P2PConnectionMessage {
    pub source: String,
    pub destination: String,
    pub action: String,
    pub c2_profile: String,
}

#[derive(Debug, Clone)]
pub struct RemoveInternalConnectionMessage {
    pub connection_uuid: String,
    pub c2_profile_name: String,
}

#[derive(Debug, Clone)]
pub struct AddInternalConnectionMessage {
    pub c2_profile_name: String,
    pub connection: ConnectionInfo,
}

#[derive(Debug, Clone)]
pub enum ConnectionInfo {
    Tcp(TcpConnectionInfo),
}

#[derive(Debug, Clone)]
pub struct TcpConnectionInfo {
    pub address: String,
    pub port: u16,
}

// ============================================================================
// Interactive Task Messages
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InteractiveTaskMessage {
    pub task_id: String,
    pub data: String,
    pub message_type: InteractiveTaskType,
}

// ============================================================================
// SOCKS Messages
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SocksMsg {
    pub server_id: u32,
    pub data: String,
    pub exit: bool,
    pub port: u32,
}

// ============================================================================
// Removed Files
// ============================================================================

#[derive(Debug, Clone, Serialize)]
pub struct RmFiles {
    pub path: String,
    pub host: String,
}

// ============================================================================
// WebSocket Message
// ============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WsMessage {
    pub data: String,
}

// ============================================================================
// Profile Trait (replaces Go Profile interface)
// ============================================================================

#[async_trait::async_trait]
pub trait Profile: Send + Sync {
    fn profile_name(&self) -> &str;
    fn is_p2p(&self) -> bool;
    async fn start(&self);
    fn stop(&self);
    fn set_sleep_interval(&self, interval: i32) -> String;
    fn get_sleep_interval(&self) -> i32;
    fn set_sleep_jitter(&self, jitter: i32) -> String;
    fn get_sleep_jitter(&self) -> i32;
    fn get_sleep_time(&self) -> i32;
    async fn sleep(&self);
    fn get_kill_date(&self) -> chrono::NaiveDate;
    fn set_encryption_key(&self, new_key: &str);
    fn get_config(&self) -> String;
    fn update_config(&self, parameter: &str, value: &str);
    fn get_push_channel(&self) -> Option<mpsc::Sender<MythicMessage>>;
    fn is_running(&self) -> bool;
}

// ============================================================================
// P2P Processor Trait (replaces Go P2PProcessor interface)
// ============================================================================

pub trait P2PProcessor: Send + Sync {
    fn profile_name(&self) -> &str;
    fn process_ingress_message_for_p2p(&self, message: &DelegateMessage);
    fn remove_internal_connection(&self, connection_uuid: &str) -> bool;
    fn add_internal_connection(&self, connection: ConnectionInfo);
    fn get_internal_p2p_map(&self) -> String;
    fn get_chunk_size(&self) -> u32;
}
