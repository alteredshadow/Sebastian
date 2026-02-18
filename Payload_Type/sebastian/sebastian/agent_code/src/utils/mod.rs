pub mod crypto;
pub mod files;
pub mod p2p;

use rand::Rng;
use std::collections::HashMap;
use std::sync::RwLock;

// ============================================================================
// Debug printing (compile-time controlled)
// ============================================================================

/// Print debug messages only when debug_mode feature is enabled
pub fn print_debug(msg: &str) {
    if cfg!(feature = "debug_mode") {
        unsafe {
            let formatted = format!("[debug] {}\n", msg);
            libc::write(2, formatted.as_ptr() as *const libc::c_void, formatted.len());
        }
        log::debug!("{}", msg);
    }
}

// ============================================================================
// Session ID generation
// ============================================================================

const SESSION_ID_CHARSET: &[u8] = b"abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789";
const SESSION_ID_LENGTH: usize = 20;

/// Generate a random 20-character alphanumeric session ID
pub fn generate_session_id() -> String {
    let mut rng = rand::thread_rng();
    (0..SESSION_ID_LENGTH)
        .map(|_| {
            let idx = rng.gen_range(0..SESSION_ID_CHARSET.len());
            SESSION_ID_CHARSET[idx] as char
        })
        .collect()
}

/// Generate a random number in the given range [min, max)
pub fn random_num_in_range(min: i32, max: i32) -> i32 {
    if min >= max {
        return min;
    }
    let mut rng = rand::thread_rng();
    rng.gen_range(min..max)
}

// ============================================================================
// In-memory file system (replaces memoryFile.go)
// ============================================================================

lazy_static::lazy_static! {
    static ref MEMORY_FILES: RwLock<HashMap<String, Vec<u8>>> = RwLock::new(HashMap::new());
}

/// Save data to the in-memory file system
pub fn save_to_memory(file_uuid: &str, data: &[u8]) {
    let mut files = MEMORY_FILES.write().expect("Memory files lock poisoned");
    files.insert(file_uuid.to_string(), data.to_vec());
}

/// Remove a file from the in-memory file system
pub fn remove_from_memory(file_uuid: &str) {
    let mut files = MEMORY_FILES.write().expect("Memory files lock poisoned");
    files.remove(file_uuid);
}

/// Get a file from the in-memory file system
pub fn get_from_memory(file_uuid: &str) -> Option<Vec<u8>> {
    let files = MEMORY_FILES.read().expect("Memory files lock poisoned");
    files.get(file_uuid).cloned()
}

// ============================================================================
// Platform-specific functions
// ============================================================================

/// Get current user
pub fn get_user() -> String {
    std::env::var("USER")
        .or_else(|_| std::env::var("LOGNAME"))
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Get hostname
pub fn get_hostname() -> String {
    hostname::get()
        .map(|h| h.to_string_lossy().to_string())
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Get current process ID
pub fn get_pid() -> i32 {
    std::process::id() as i32
}

/// Get current working directory
pub fn get_cwd() -> String {
    std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "/".to_string())
}

/// Get process name
pub fn get_process_name() -> String {
    std::env::current_exe()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_else(|| "unknown".to_string())
        })
        .unwrap_or_else(|_| "unknown".to_string())
}

/// Get all non-loopback IP addresses
pub fn get_current_ip_address() -> Vec<String> {
    match local_ip_address::list_afinet_netifas() {
        Ok(interfaces) => {
            let mut ips: Vec<String> = interfaces
                .into_iter()
                .filter(|(_, ip)| !ip.is_loopback())
                .map(|(_, ip)| ip.to_string())
                .collect();
            ips.sort();
            ips
        }
        Err(_) => vec!["127.0.0.1".to_string()],
    }
}

/// Get OS description
#[cfg(target_os = "macos")]
pub fn get_os() -> String {
    let _info = sysinfo::System::new_all();
    format!(
        "macOS {}",
        sysinfo::System::os_version().unwrap_or_else(|| "Unknown".to_string())
    )
}

#[cfg(target_os = "linux")]
pub fn get_os() -> String {
    format!(
        "Linux {}",
        sysinfo::System::os_version().unwrap_or_else(|| "Unknown".to_string())
    )
}

/// Get CPU architecture
pub fn get_architecture() -> String {
    std::env::consts::ARCH.to_string()
}

/// Get domain from Kerberos config (matches Poseidon behavior)
pub fn get_domain() -> String {
    if let Ok(contents) = std::fs::read_to_string("/etc/krb5.conf") {
        for line in contents.lines() {
            if line.contains("default_realm") {
                if let Some(realm) = line.split('=').nth(1) {
                    let realm = realm.trim();
                    if !realm.is_empty() {
                        return realm.to_string();
                    }
                }
            }
        }
    }
    String::new()
}

/// Check if running as elevated (root/admin)
pub fn is_elevated() -> bool {
    nix::unistd::geteuid().is_root()
}

/// Get effective user
pub fn get_effective_user() -> String {
    nix::unistd::User::from_uid(nix::unistd::geteuid())
        .ok()
        .flatten()
        .map(|u| u.name)
        .unwrap_or_else(|| "unknown".to_string())
}
