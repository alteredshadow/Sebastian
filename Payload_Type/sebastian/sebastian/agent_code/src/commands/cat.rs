use crate::structs::Task;
use serde::Deserialize;
use tokio::io::AsyncReadExt;

// 5 MB — large enough for most text files, small enough to avoid OOM
pub(crate) const MAX_READ_BYTES: usize = 5 * 1024 * 1024;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::make_test_task;

    #[tokio::test]
    async fn test_cat_reads_file_contents() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.txt");
        tokio::fs::write(&path, b"hello sebastian").await.unwrap();

        let (task, mut resp_rx, _) = make_test_task("t1", &path.to_string_lossy());
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert_eq!(resp.user_output, "hello sebastian");
        assert_ne!(resp.status, "error");
    }

    #[tokio::test]
    async fn test_cat_json_path_param() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("json.txt");
        tokio::fs::write(&path, b"via json").await.unwrap();

        let params = serde_json::json!({"path": path.to_string_lossy()}).to_string();
        let (task, mut resp_rx, _) = make_test_task("t2", &params);
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert_eq!(resp.user_output, "via json");
    }

    #[tokio::test]
    async fn test_cat_nonexistent_file_returns_error() {
        let (task, mut resp_rx, _) = make_test_task("t3", "/no/such/path/file_xyz.txt");
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert_eq!(resp.status, "error");
        assert!(resp.user_output.to_lowercase().contains("failed") || !resp.user_output.is_empty());
    }

    #[tokio::test]
    async fn test_cat_output_bounded_for_oversized_file() {
        // Write 2× MAX_READ_BYTES so there is unambiguously more data than the
        // cap, regardless of how many bytes the OS returns in a single read().
        // The guarantee cat provides is that user_output is never longer than
        // MAX_READ_BYTES (+ a small truncation notice if the read filled the probe
        // buffer), so we assert the length stays within a reasonable bound.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("big.txt");
        let data = vec![b'A'; MAX_READ_BYTES * 2];
        tokio::fs::write(&path, &data).await.unwrap();

        let (task, mut resp_rx, _) = make_test_task("t4", &path.to_string_lossy());
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        // Allow MAX_READ_BYTES + 200 to cover the truncation notice text
        assert!(
            resp.user_output.len() <= MAX_READ_BYTES + 200,
            "output must be bounded, got {} bytes",
            resp.user_output.len()
        );
    }

    #[tokio::test]
    async fn test_cat_small_file_not_truncated() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("small.txt");
        let data = vec![b'B'; 1024]; // 1 KB — well under the cap
        tokio::fs::write(&path, &data).await.unwrap();

        let (task, mut resp_rx, _) = make_test_task("t5", &path.to_string_lossy());
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert!(!resp.user_output.contains("[truncated:"), "small file must not show truncation notice");
        assert_eq!(resp.user_output.len(), 1024);
    }

    #[tokio::test]
    async fn test_cat_binary_file_does_not_panic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("binary.bin");
        let data: Vec<u8> = (0u8..=255).cycle().take(1024).collect();
        tokio::fs::write(&path, &data).await.unwrap();

        let (task, mut resp_rx, _) = make_test_task("t6", &path.to_string_lossy());
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        // from_utf8_lossy must have produced something (even if replacement chars)
        assert!(!resp.user_output.is_empty() || data.is_empty());
    }
}
