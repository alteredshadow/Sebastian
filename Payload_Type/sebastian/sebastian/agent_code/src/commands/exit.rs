use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    response.user_output = "Exiting...".to_string();
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;

    // Give the poll loop a brief window to flush the response
    tokio::time::sleep(tokio::time::Duration::from_millis(500)).await;

    std::process::exit(0);
}
