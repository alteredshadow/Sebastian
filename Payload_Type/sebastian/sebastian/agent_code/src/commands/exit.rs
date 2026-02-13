use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    response.user_output = "Exiting...".to_string();
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;

    // Send exit message to Mythic so it can immediately show the agent as dead
    crate::responses::send_exit_message(crate::profiles::get_push_channel).await;

    // Give time for messages to be sent
    tokio::time::sleep(tokio::time::Duration::from_secs(3)).await;

    crate::utils::print_debug("Exit command terminating agent");
    std::process::exit(0);
}
