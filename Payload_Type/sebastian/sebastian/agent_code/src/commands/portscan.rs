use crate::structs::Task;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tokio::net::TcpStream;
use tokio::time::{timeout, Duration};

#[derive(Deserialize)]
struct PortscanArgs {
    hosts: Vec<String>,
    ports: Vec<u16>,
    #[serde(default = "default_timeout")]
    timeout_ms: u64,
}

fn default_timeout() -> u64 { 500 }

#[derive(Serialize)]
struct CidrResult {
    range: String,
    hosts: Vec<HostResult>,
}

#[derive(Serialize)]
struct HostResult {
    ip: String,
    hostname: String,
    pretty_name: String,
    open_ports: Vec<u16>,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: PortscanArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let timeout_duration = Duration::from_millis(args.timeout_ms);

    // Group results by host, matching Poseidon's CIDR/host structure
    let mut host_ports: HashMap<String, Vec<u16>> = HashMap::new();

    for host in &args.hosts {
        for port in &args.ports {
            if task.should_stop() { break; }
            let addr = format!("{}:{}", host, port);
            let is_open = timeout(timeout_duration, TcpStream::connect(&addr))
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false);

            if is_open {
                host_ports.entry(host.clone()).or_default().push(*port);
            }
        }
    }

    // Build CIDR result structure for browser script
    let mut hosts = Vec::new();
    for (host, ports) in &host_ports {
        hosts.push(HostResult {
            ip: host.clone(),
            hostname: host.clone(),
            pretty_name: host.clone(),
            open_ports: ports.clone(),
        });
    }
    let results = vec![CidrResult {
        range: "scan".to_string(),
        hosts,
    }];

    response.user_output = serde_json::to_string_pretty(&results)
        .unwrap_or_else(|_| "[]".to_string());
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
