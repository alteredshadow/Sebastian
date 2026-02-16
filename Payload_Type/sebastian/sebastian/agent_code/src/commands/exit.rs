use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    response.user_output = "Exiting...".to_string();
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;

    // Send exit message to Mythic so it can immediately show the agent as dead.
    // For push profiles this sends immediately. For poll profiles this sets a flag
    // so the next poll cycle sends action="exit".
    crate::responses::send_exit_message(crate::profiles::get_push_channel).await;

    // Wait long enough for the poll cycle to pick up the exit message.
    // Use the current sleep interval + buffer to ensure at least one poll happens.
    let sleep_secs = crate::profiles::get_sleep_time().max(1) as u64 + 3;
    tokio::time::sleep(tokio::time::Duration::from_secs(sleep_secs)).await;

    crate::utils::print_debug("Exit command terminating agent");
    std::process::exit(0);
}
