use crate::structs::Task;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

// 5 MB — large enough for most text files, small enough to avoid OOM
const MAX_READ_BYTES: usize = 5 * 1024 * 1024;

#[derive(Deserialize)]
struct CatArgs {
    path: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: CatArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => CatArgs { path: task.data.params.clone() },
    };

    let mut file = match tokio::fs::File::open(&args.path).await {
        Ok(f) => f,
        Err(e) => {
            response.set_error(&format!("Failed to open file: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut buf = vec![0u8; MAX_READ_BYTES + 1];
    let n = match file.read(&mut buf).await {
        Ok(n) => n,
        Err(e) => {
            response.set_error(&format!("Failed to read file: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let truncated = n > MAX_READ_BYTES;
    let data = &buf[..std::cmp::min(n, MAX_READ_BYTES)];
    let mut contents = String::from_utf8_lossy(data).into_owned();
    if truncated {
        contents.push_str(&format!("\n\n[truncated: output exceeded {} MB]", MAX_READ_BYTES / 1024 / 1024));
    }

    response.user_output = contents;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
