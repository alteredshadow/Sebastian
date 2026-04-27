use crate::structs::Task;
use tokio::process::Command;


pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let command_str = task.data.params.clone();

    match Command::new("/bin/sh")
        .arg("-c")
        .arg(&command_str)
        .output()
        .await
    {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = format!("{}{}", stdout, stderr);
            response.completed = true;
        }
        Err(e) => {
            response.set_error(&format!("Failed to execute shell command: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::make_test_task;

    #[tokio::test]
    async fn test_shell_stdout_captured() {
        let (task, mut resp_rx, _) = make_test_task("t1", "echo hello_sebastian");
        execute(task).await;
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert!(resp.user_output.contains("hello_sebastian"));
    }

    #[tokio::test]
    async fn test_shell_stderr_captured() {
        let (task, mut resp_rx, _) = make_test_task("t2", "echo stderr_msg >&2");
        execute(task).await;
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert!(resp.user_output.contains("stderr_msg"));
    }

    #[tokio::test]
    async fn test_shell_stdout_and_stderr_combined() {
        let (task, mut resp_rx, _) =
            make_test_task("t3", "echo out_part; echo err_part >&2");
        execute(task).await;
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert!(resp.user_output.contains("out_part"));
        assert!(resp.user_output.contains("err_part"));
    }

    #[tokio::test]
    async fn test_shell_nonzero_exit_still_completes() {
        // A command that exits non-zero must still produce a completed response
        let (task, mut resp_rx, _) = make_test_task("t4", "exit 42");
        execute(task).await;
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
    }

    #[tokio::test]
    async fn test_shell_empty_output_is_ok() {
        let (task, mut resp_rx, _) = make_test_task("t5", "true");
        execute(task).await;
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        assert_ne!(resp.status, "error");
    }

    #[tokio::test]
    async fn test_shell_multiline_output() {
        let (task, mut resp_rx, _) =
            make_test_task("t6", "printf 'line1\\nline2\\nline3\\n'");
        execute(task).await;
        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        let lines: Vec<_> = resp.user_output.lines().collect();
        assert!(lines.len() >= 3);
    }

    #[tokio::test]
    async fn test_shell_remove_task_sent() {
        // Ensure remove_running_task is always sent so the task registry is cleaned up
        let (task, _, mut remove_rx) = make_test_task("t7", "echo ok");
        execute(task).await;
        let id = remove_rx.recv().await.unwrap();
        assert_eq!(id, "t7");
    }
}
