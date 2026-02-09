use crate::structs::Task;
use serde::Deserialize;
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

    let mut results = Vec::new();
    let timeout_duration = Duration::from_millis(args.timeout_ms);

    for host in &args.hosts {
        for port in &args.ports {
            if task.should_stop() { break; }
            let addr = format!("{}:{}", host, port);
            let is_open = timeout(timeout_duration, TcpStream::connect(&addr))
                .await
                .map(|r| r.is_ok())
                .unwrap_or(false);

            if is_open {
                results.push(format!("{}:{} - OPEN", host, port));
            }
        }
    }

    response.user_output = if results.is_empty() {
        "No open ports found".to_string()
    } else {
        results.join("\n")
    };
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
