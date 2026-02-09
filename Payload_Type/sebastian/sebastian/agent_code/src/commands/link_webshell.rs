use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct LinkWebshellArgs {
    url: String,
    #[serde(default = "default_profile")]
    c2_profile_name: String,
}

fn default_profile() -> String { "webshell".to_string() }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: LinkWebshellArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // TODO: Implement webshell P2P linking via HTTP
    response.user_output = format!("Linking to webshell at {}", args.url);
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
