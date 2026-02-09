use crate::structs::Task;
use serde::Deserialize;
use std::path::Path;

#[derive(Deserialize)]
struct TriageArgs {
    path: String,
    #[serde(default = "default_depth")]
    max_depth: usize,
}

fn default_depth() -> usize { 3 }

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: TriageArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut output = String::new();
    triage_dir(Path::new(&args.path), 0, args.max_depth, &mut output);
    response.user_output = output;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}

fn triage_dir(path: &Path, depth: usize, max_depth: usize, output: &mut String) {
    if depth > max_depth { return; }
    let indent = "  ".repeat(depth);
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let meta = entry.metadata();
            let size = meta.as_ref().map(|m| m.len()).unwrap_or(0);
            let is_dir = meta.as_ref().map(|m| m.is_dir()).unwrap_or(false);
            let name = entry.file_name().to_string_lossy().to_string();
            if is_dir {
                output.push_str(&format!("{}{}/\n", indent, name));
                triage_dir(&entry.path(), depth + 1, max_depth, output);
            } else {
                output.push_str(&format!("{}{} ({} bytes)\n", indent, name, size));
            }
        }
    }
}
