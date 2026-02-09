use crate::structs::{RemoveInternalConnectionMessage, Task};
use serde::Deserialize;

#[derive(Deserialize)]
struct UnlinkWebshellArgs {
    connection_id: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: UnlinkWebshellArgs = match serde_json::from_str(&task.data.params) {
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
        c2_profile_name: "webshell".to_string(),
    };
    let _ = task.job.remove_internal_connection_channel.send(msg).await;
    response.user_output = format!("Unlinked webshell: {}", args.connection_id);
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
