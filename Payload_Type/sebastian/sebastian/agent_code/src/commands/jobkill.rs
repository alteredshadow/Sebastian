use crate::structs::Task;
use crate::tasks;
use serde::Deserialize;

#[derive(Deserialize)]
struct JobkillArgs { id: String }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: JobkillArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => JobkillArgs { id: task.data.params.trim().to_string() },
    };

    if tasks::kill_task(&args.id) {
        response.user_output = format!("Killed task: {}", args.id);
    } else {
        response.user_output = format!("Task not found: {}", args.id);
    }
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
