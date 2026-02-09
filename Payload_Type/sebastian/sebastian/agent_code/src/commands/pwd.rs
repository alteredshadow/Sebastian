use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    match std::env::current_dir() {
        Ok(path) => {
            response.user_output = path.to_string_lossy().to_string();
            response.completed = true;
        }
        Err(e) => {
            response.set_error(&format!("Failed to get current directory: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
