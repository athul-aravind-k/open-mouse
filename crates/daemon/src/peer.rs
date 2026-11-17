use crate::config::AppConfig;
use crate::discovery::{should_initiate_connection, DiscoveryService};
use crate::edge::{DisplayArrangement, EdgeDirection};
use crate::state::{KvmStateMachine, StateAction, StateEvent};
use capture::{
    get_cursor_pos, get_primary_screen_size, park_cursor, set_cursor_pos, unclip_cursor,
    CaptureEngine, CapturedEvent,
};
use crossbeam_channel::{Receiver, Sender};
use emulate::InputInjector;
use net::{QuicEndpoint, QuicSession};
use proto::{Packet, ScreenDescriptor};
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, PartialEq)]
pub enum PeerStatus {
    SearchingOnLan { port: u16 },
    WaitingForPeer { local_ip: String, port: u16 },
    Connecting { target: String },
    Connected {
        peer_name: String,
        peer_addr: String,
        rtt_ms: Option<f32>,
        fingerprint: Option<String>,
    },
    ControllingRemote,
    RemoteControlled,
}

pub struct PeerNode {
    config: Arc<Mutex<AppConfig>>,
    status: Arc<Mutex<PeerStatus>>,
    status_rx: Receiver<PeerStatus>,
    identify_rx: Receiver<(u32, u32)>,
    remote_screen: Arc<Mutex<Option<ScreenDescriptor>>>,
    cmd_tx: Sender<NodeEvent>,
    running: Arc<AtomicBool>,
}

impl PeerNode {
    pub fn new(initial_config: AppConfig) -> Self {
        let (status_tx, status_rx) = crossbeam_channel::unbounded();
        let (identify_tx, identify_rx) = crossbeam_channel::unbounded();
        let (event_tx, event_rx) = crossbeam_channel::unbounded();
        let remote_screen = Arc::new(Mutex::new(None));
        let running = Arc::new(AtomicBool::new(true));
        let config = Arc::new(Mutex::new(initial_config));

        let initial_status = {
            let cfg = config.lock().unwrap();
            if cfg.peer_ip.is_empty() {
                PeerStatus::SearchingOnLan { port: cfg.peer_port }
            } else {
                PeerStatus::WaitingForPeer {
                    local_ip: get_local_ip(),
                    port: cfg.peer_port,
                }
            }
        };
        let status = Arc::new(Mutex::new(initial_status.clone()));
        let _ = status_tx.send(initial_status);

        let config_clone = config.clone();
        let status_clone = status.clone();
        let running_clone = running.clone();
        let event_tx_clone = event_tx.clone();
        let remote_screen_clone = remote_screen.clone();

        thread::spawn(move || {
            run_peer_lifecycle(
                config_clone,
                status_clone,
                status_tx,
                event_tx_clone,
                event_rx,
                identify_tx,
                remote_screen_clone,
                running_clone,
            );
        });

        Self {
            config,
            status,
            status_rx,
            identify_rx,
            remote_screen,
            cmd_tx: event_tx,
            running,
        }
    }

    pub fn get_status(&self) -> PeerStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn status_receiver(&self) -> Receiver<PeerStatus> {
        self.status_rx.clone()
    }

    pub fn identify_receiver(&self) -> Receiver<(u32, u32)> {
        self.identify_rx.clone()
    }

    pub fn get_remote_screen(&self) -> Option<ScreenDescriptor> {
        self.remote_screen.lock().unwrap().clone()
    }

    pub fn get_config(&self) -> AppConfig {
        self.config.lock().unwrap().clone()
    }

    pub fn update_peer_ip(&self, new_ip: String) {
        let mut cfg = self.config.lock().unwrap();
        cfg.peer_ip = new_ip;
        let _ = crate::config::save_config(&cfg);
    }

    pub fn update_arrangement(&self, arrangement: DisplayArrangement) {
        {
            let mut cfg = self.config.lock().unwrap();
            cfg.shared_edge = arrangement.relative_edge;
            cfg.display_arrangement = arrangement.clone();
            let _ = crate::config::save_config(&cfg);
        }
        let _ = self.cmd_tx.send(NodeEvent::UpdateArrangement(arrangement));
    }

    pub fn trigger_identify(&self) {
        let _ = self.cmd_tx.send(NodeEvent::TriggerIdentify(2500));
    }

    pub fn toggle_edge(&self) -> EdgeDirection {
        let mut cfg = self.config.lock().unwrap();
        let new_edge = match cfg.shared_edge {
            EdgeDirection::Right => EdgeDirection::Left,
            EdgeDirection::Left => EdgeDirection::Right,
            _ => EdgeDirection::Right,
        };
        cfg.shared_edge = new_edge;
        cfg.display_arrangement.relative_edge = new_edge;
        let arrangement = cfg.display_arrangement.clone();
        let _ = crate::config::save_config(&cfg);
        let _ = self.cmd_tx.send(NodeEvent::UpdateArrangement(arrangement));
        new_edge
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for PeerNode {
    fn drop(&mut self) {
        self.stop();
    }
}

enum NodeEvent {
    LocalInput(CapturedEvent),
    Network(Packet),
    NetworkDisconnected,
    NewQuicSession {
        outgoing_tx: Sender<(Packet, bool)>,
        peer_addr: String,
    },
    RttUpdated(f32),
    ClipboardChanged,
    UpdateArrangement(DisplayArrangement),
    TriggerIdentify(u32),
}

fn run_peer_lifecycle(
    config: Arc<Mutex<AppConfig>>,
    status: Arc<Mutex<PeerStatus>>,
    status_tx: Sender<PeerStatus>,
    event_tx: Sender<NodeEvent>,
    event_rx: Receiver<NodeEvent>,
    identify_tx: Sender<(u32, u32)>,
    remote_screen: Arc<Mutex<Option<ScreenDescriptor>>>,
    running: Arc<AtomicBool>,
) {
    let (machine_id, device_name, port, trusted_fps) = {
        let cfg = config.lock().unwrap();
        (
            cfg.machine_id.clone(),
            cfg.device_name.clone(),
            cfg.peer_port,
            cfg.trusted_fingerprints.clone(),
        )
    };

    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(r) => r,
        Err(e) => {
            eprintln!("[PeerNode] Failed to build tokio runtime: {}", e);
            return;
        }
    };

    let bind_addr: SocketAddr = match format!("0.0.0.0:{}", port).parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("[PeerNode] Invalid bind address: {}", e);
            return;
        }
    };

    let endpoint = match QuicEndpoint::new(bind_addr, &device_name, trusted_fps) {
        Ok(ep) => Arc::new(ep),
        Err(e) => {
            eprintln!("[PeerNode] Failed to bind QUIC endpoint on {}: {}", bind_addr, e);
            return;
        }
    };

    // Initialize UDP LAN Auto-Discovery on port 48898
    let discovery = Arc::new(DiscoveryService::new(
        machine_id.clone(),
        device_name.clone(),
        port,
    ));

    // 1. Inbound QUIC accept task
    let ep_inbound = endpoint.clone();
    let event_tx_inbound = event_tx.clone();
    let running_inbound = running.clone();
    rt.spawn(async move {
        while running_inbound.load(Ordering::SeqCst) {
            if let Some(session) = ep_inbound.accept().await {
                let peer_addr = session.remote_addr().to_string();
                let (out_tx, out_rx) = crossbeam_channel::unbounded::<(Packet, bool)>();
                let event_tx_reader = event_tx_inbound.clone();
                let running_sess = running_inbound.clone();

                tokio::spawn(async move {
                    run_session_worker(session, out_rx, event_tx_reader, running_sess).await;
                });

                let _ = event_tx_inbound.send(NodeEvent::NewQuicSession {
                    outgoing_tx: out_tx,
                    peer_addr,
                });
            }
        }
    });

    // 2. Outbound QUIC connection task (Manual IP or UDP Auto-Discovered)
    let ep_outbound = endpoint.clone();
    let config_outbound = config.clone();
    let event_tx_outbound = event_tx.clone();
    let running_outbound = running.clone();
    let discovery_peer_rx = discovery.peer_receiver();
    let local_id = machine_id.clone();

    rt.spawn(async move {
        while running_outbound.load(Ordering::SeqCst) {
            let (manual_ip, port) = {
                let cfg = config_outbound.lock().unwrap();
                (cfg.peer_ip.clone(), cfg.peer_port)
            };

            if !manual_ip.is_empty() && manual_ip != "127.0.0.1" && manual_ip != "0.0.0.0" {
                if let Ok(target) = format!("{}:{}", manual_ip, port).parse::<SocketAddr>() {
                    if let Ok(session) = ep_outbound.connect(target).await {
                        let peer_addr = target.to_string();
                        let (out_tx, out_rx) = crossbeam_channel::unbounded::<(Packet, bool)>();
                        let event_tx_reader = event_tx_outbound.clone();
                        let running_sess = running_outbound.clone();

                        tokio::spawn(async move {
                            run_session_worker(session, out_rx, event_tx_reader, running_sess).await;
                        });

                        let _ = event_tx_outbound.send(NodeEvent::NewQuicSession {
                            outgoing_tx: out_tx,
                            peer_addr,
                        });
                        tokio::time::sleep(Duration::from_secs(5)).await;
                    }
                }
                tokio::time::sleep(Duration::from_secs(3)).await;
            } else {
                match discovery_peer_rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(peer) => {
                        if should_initiate_connection(&local_id, &peer.machine_id) {
                            if let Ok(target) = format!("{}:{}", peer.ip, peer.port).parse::<SocketAddr>() {
                                if let Ok(session) = ep_outbound.connect(target).await {
                                    let peer_addr = target.to_string();
                                    let (out_tx, out_rx) = crossbeam_channel::unbounded::<(Packet, bool)>();
                                    let event_tx_reader = event_tx_outbound.clone();
                                    let running_sess = running_outbound.clone();

                                    tokio::spawn(async move {
                                        run_session_worker(session, out_rx, event_tx_reader, running_sess).await;
                                    });

                                    let _ = event_tx_outbound.send(NodeEvent::NewQuicSession {
                                        outgoing_tx: out_tx,
                                        peer_addr,
                                    });
                                    tokio::time::sleep(Duration::from_secs(5)).await;
                                }
                            }
                        }
                    }
                    Err(_) => {}
                }
            }
        }
    });

    // 3. Local input capture engine
    let (capture, capture_rx) = CaptureEngine::new();
    let event_tx_capture = event_tx.clone();
    let running_capture = running.clone();
    thread::spawn(move || {
        while running_capture.load(Ordering::SeqCst) {
            if let Ok(evt) = capture_rx.recv() {
                if event_tx_capture.send(NodeEvent::LocalInput(evt)).is_err() {
                    break;
                }
            }
        }
    });

    // 4. Shared clipboard listener and manager
    let _clipboard_listener = clipboard::ClipboardListener::new();
    let clipboard_rx = _clipboard_listener.receiver();
    let mut clipboard_manager = clipboard::ClipboardManager::new();
    let event_tx_clipboard = event_tx.clone();
    let running_clipboard = running.clone();
    thread::spawn(move || {
        while running_clipboard.load(Ordering::SeqCst) {
            match clipboard_rx.recv_timeout(Duration::from_millis(200)) {
                Ok(_) => {
                    if event_tx_clipboard.send(NodeEvent::ClipboardChanged).is_err() {
                        break;
                    }
                }
                Err(crossbeam_channel::RecvTimeoutError::Timeout) => {}
                Err(crossbeam_channel::RecvTimeoutError::Disconnected) => break,
            }
        }
    });

    let (screen_w, screen_h) = get_primary_screen_size();
    let mut state_machine = KvmStateMachine::new(screen_w, screen_h);
    {
        let cfg = config.lock().unwrap();
        state_machine.set_arrangement(cfg.display_arrangement.clone());
    }
    let mut injector = InputInjector::new();

    let mut active_outgoing_tx: Option<Sender<(Packet, bool)>> = None;
    let mut current_peer_name = String::new();
    let mut current_peer_addr = String::new();
    let mut current_rtt_ms: Option<f32> = None;
    let mut current_fingerprint: Option<String> = None;
    let mut is_controlled = false;
    let mut entry_time = Instant::now();
    let entry_lockout = Duration::from_millis(150);

    while running.load(Ordering::SeqCst) {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(NodeEvent::NewQuicSession { outgoing_tx, peer_addr }) => {
                if active_outgoing_tx.is_none() {
                    // Send Hello packet
                    let (device_name, my_fp) = {
                        let cfg = config.lock().unwrap();
                        (cfg.device_name.clone(), endpoint.cert_fingerprint().to_string())
                    };

                    let hello = Packet::Hello {
                        version: 1,
                        machine_name: device_name,
                        screen: ScreenDescriptor {
                            machine_id: machine_id.clone(),
                            width_px: screen_w as u32,
                            height_px: screen_h as u32,
                            dpi_scale: 1.0,
                            offset_x: 0,
                            offset_y: 0,
                        },
                        cert_fingerprint: my_fp,
                    };
                    let _ = outgoing_tx.send((hello, false));

                    active_outgoing_tx = Some(outgoing_tx);
                    current_peer_name = peer_addr.clone();
                    current_peer_addr = peer_addr.clone();
                    state_machine.process_event(StateEvent::Connected);
                    discovery.set_connected(true);

                    let new_status = PeerStatus::Connected {
                        peer_name: peer_addr.clone(),
                        peer_addr,
                        rtt_ms: None,
                        fingerprint: None,
                    };
                    *status.lock().unwrap() = new_status.clone();
                    let _ = status_tx.send(new_status);
                }
            }

            Ok(NodeEvent::RttUpdated(rtt)) => {
                current_rtt_ms = Some(rtt);
                if let PeerStatus::Connected { .. } = *status.lock().unwrap() {
                    let st = PeerStatus::Connected {
                        peer_name: current_peer_name.clone(),
                        peer_addr: current_peer_addr.clone(),
                        rtt_ms: current_rtt_ms,
                        fingerprint: current_fingerprint.clone(),
                    };
                    *status.lock().unwrap() = st.clone();
                    let _ = status_tx.send(st);
                }
            }

            Ok(NodeEvent::LocalInput(CapturedEvent::EdgeTrigger { x, y })) => {
                if active_outgoing_tx.is_some() && !state_machine.is_remote_active() {
                    let action = state_machine.process_event(StateEvent::EdgeHit { x, y });
                    if let StateAction::EnterRemote { park_x, park_y, x_ratio, y_ratio } = action {
                        capture.set_swallowing(true);
                        park_cursor(park_x, park_y);

                        if let Some(ref tx) = active_outgoing_tx {
                            let enter_pkt = Packet::Enter { x_ratio, y_ratio };
                            let _ = tx.send((enter_pkt, false));

                            let st = PeerStatus::ControllingRemote;
                            *status.lock().unwrap() = st.clone();
                            let _ = status_tx.send(st);
                        }
                    }
                }
            }

            Ok(NodeEvent::LocalInput(CapturedEvent::Packet(pkt))) => {
                if state_machine.is_remote_active() {
                    if let Some(ref tx) = active_outgoing_tx {
                        let is_unreliable = matches!(pkt, Packet::MouseMoveRel { .. });
                        let _ = tx.send((pkt, is_unreliable));
                    }
                }
            }

            Ok(NodeEvent::LocalInput(CapturedEvent::EmergencyEscape)) => {
                capture.set_swallowing(false);
                unclip_cursor();
                set_cursor_pos(screen_w / 2, screen_h / 2);
                injector.release_all();
                state_machine.process_event(StateEvent::EmergencyEscape);

                if let Some(ref tx) = active_outgoing_tx {
                    let _ = tx.send((Packet::EmergencyReset, false));
                }

                let st = PeerStatus::Connected {
                    peer_name: current_peer_name.clone(),
                    peer_addr: current_peer_addr.clone(),
                    rtt_ms: current_rtt_ms,
                    fingerprint: current_fingerprint.clone(),
                };
                *status.lock().unwrap() = st.clone();
                let _ = status_tx.send(st);
            }

            Ok(NodeEvent::Network(pkt)) => {
                match pkt {
                    Packet::Hello { machine_name, screen, cert_fingerprint, .. } => {
                        *remote_screen.lock().unwrap() = Some(screen.clone());
                        state_machine.set_remote_screen_size(screen.width_px, screen.height_px);
                        current_peer_name = machine_name.clone();
                        current_fingerprint = Some(cert_fingerprint.clone());

                        // TOFU: pin fingerprint in configuration
                        {
                            let mut cfg = config.lock().unwrap();
                            if !cfg.trusted_fingerprints.contains(&cert_fingerprint) {
                                cfg.trusted_fingerprints.push(cert_fingerprint.clone());
                                let _ = crate::config::save_config(&cfg);
                            }
                        }

                        let st = PeerStatus::Connected {
                            peer_name: machine_name,
                            peer_addr: current_peer_addr.clone(),
                            rtt_ms: current_rtt_ms,
                            fingerprint: current_fingerprint.clone(),
                        };
                        *status.lock().unwrap() = st.clone();
                        let _ = status_tx.send(st);
                    }

                    Packet::Heartbeat { timestamp_ms } => {
                        let now_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        let ack = Packet::HeartbeatAck {
                            timestamp_ms: now_ms,
                            echoed_timestamp_ms: timestamp_ms,
                        };
                        if let Some(ref tx) = active_outgoing_tx {
                            let _ = tx.send((ack, false));
                        }
                    }

                    Packet::HeartbeatAck { echoed_timestamp_ms, .. } => {
                        let now_ms = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis() as u64;
                        if now_ms >= echoed_timestamp_ms {
                            let rtt = (now_ms - echoed_timestamp_ms) as f32;
                            current_rtt_ms = Some(rtt);
                            let st = PeerStatus::Connected {
                                peer_name: current_peer_name.clone(),
                                peer_addr: current_peer_addr.clone(),
                                rtt_ms: current_rtt_ms,
                                fingerprint: current_fingerprint.clone(),
                            };
                            *status.lock().unwrap() = st.clone();
                            let _ = status_tx.send(st);
                        }
                    }

                    Packet::Enter { x_ratio, y_ratio } => {
                        let action = state_machine.process_event(StateEvent::RemoteEnterReceived { x_ratio, y_ratio });
                        if let StateAction::AcceptRemoteControl { target_x, target_y } = action {
                            set_cursor_pos(target_x, target_y);
                            is_controlled = true;
                            entry_time = Instant::now();

                            let st = PeerStatus::RemoteControlled;
                            *status.lock().unwrap() = st.clone();
                            let _ = status_tx.send(st);
                        }
                    }

                    Packet::Leave { x_ratio, y_ratio } => {
                        let action = state_machine.process_event(StateEvent::RemoteLeaveReceived { x_ratio, y_ratio });
                        if let StateAction::ReturnToLocal { return_x, return_y } = action {
                            capture.set_swallowing(false);
                            unclip_cursor();
                            set_cursor_pos(return_x, return_y);

                            let st = PeerStatus::Connected {
                                peer_name: current_peer_name.clone(),
                                peer_addr: current_peer_addr.clone(),
                                rtt_ms: current_rtt_ms,
                                fingerprint: current_fingerprint.clone(),
                            };
                            *status.lock().unwrap() = st.clone();
                            let _ = status_tx.send(st);
                        }
                    }

                    Packet::MouseMoveRel { dx, dy } => {
                        if is_controlled {
                            injector.move_rel(dx, dy);

                            if entry_time.elapsed() >= entry_lockout {
                                let (cur_x, cur_y) = get_cursor_pos();
                                if let Some((x_ratio, y_ratio)) = state_machine
                                    .arrangement()
                                    .check_return_boundary(cur_x, cur_y, screen_w, screen_h)
                                {
                                    let action = state_machine.process_event(StateEvent::ReturnBoundaryHit {
                                        x_ratio,
                                        y_ratio,
                                    });
                                    if let StateAction::SendReturnToPeer { x_ratio, y_ratio } = action {
                                        is_controlled = false;
                                        injector.release_all();

                                        if let Some(ref tx) = active_outgoing_tx {
                                            let _ = tx.send((Packet::Leave { x_ratio, y_ratio }, false));
                                        }

                                        let st = PeerStatus::Connected {
                                            peer_name: current_peer_name.clone(),
                                            peer_addr: current_peer_addr.clone(),
                                            rtt_ms: current_rtt_ms,
                                            fingerprint: current_fingerprint.clone(),
                                        };
                                        *status.lock().unwrap() = st.clone();
                                        let _ = status_tx.send(st);
                                    }
                                }
                            }
                        }
                    }

                    Packet::MouseButton { button, pressed } => {
                        if is_controlled {
                            injector.mouse_button(button, pressed);
                        }
                    }

                    Packet::MouseScroll { delta_x, delta_y } => {
                        if is_controlled {
                            injector.mouse_scroll(delta_x, delta_y);
                        }
                    }

                    Packet::Key { scancode, extended, pressed } => {
                        if is_controlled {
                            injector.key(scancode, extended, pressed);
                        }
                    }

                    Packet::EmergencyReset => {
                        capture.set_swallowing(false);
                        unclip_cursor();
                        injector.release_all();
                        is_controlled = false;
                        state_machine.process_event(StateEvent::EmergencyResetReceived);

                        let st = PeerStatus::Connected {
                            peer_name: current_peer_name.clone(),
                            peer_addr: current_peer_addr.clone(),
                            rtt_ms: current_rtt_ms,
                            fingerprint: current_fingerprint.clone(),
                        };
                        *status.lock().unwrap() = st.clone();
                        let _ = status_tx.send(st);
                    }

                    Packet::ClipboardData { format, hash, payload } => {
                        if let Err(e) = clipboard_manager.inject_remote(format, hash, &payload) {
                            tracing::warn!("Failed to inject remote clipboard: {}", e);
                        }
                    }

                    Packet::Identify { display_number, duration_ms } => {
                        let _ = identify_tx.send((display_number, duration_ms));
                    }

                    _ => {}
                }
            }

            Ok(NodeEvent::ClipboardChanged) => {
                if let Some((format, hash, payload)) = clipboard_manager.poll_local_change() {
                    if let Some(ref tx) = active_outgoing_tx {
                        let pkt = Packet::ClipboardData { format, hash, payload };
                        let _ = tx.send((pkt, false));
                    }
                }
            }

            Ok(NodeEvent::UpdateArrangement(arr)) => {
                state_machine.set_arrangement(arr.clone());
                if let Some(ref tx) = active_outgoing_tx {
                    let pkt = Packet::LayoutUpdate {
                        screens: vec![ScreenDescriptor {
                            machine_id: machine_id.clone(),
                            width_px: screen_w as u32,
                            height_px: screen_h as u32,
                            dpi_scale: 1.0,
                            offset_x: arr.offset_x,
                            offset_y: arr.offset_y,
                        }],
                    };
                    let _ = tx.send((pkt, false));
                }
            }

            Ok(NodeEvent::TriggerIdentify(duration_ms)) => {
                let _ = identify_tx.send((1, duration_ms));
                if let Some(ref tx) = active_outgoing_tx {
                    let pkt = Packet::Identify {
                        display_number: 2,
                        duration_ms,
                    };
                    let _ = tx.send((pkt, false));
                }
            }

            Ok(NodeEvent::NetworkDisconnected) => {
                active_outgoing_tx = None;
                is_controlled = false;
                capture.set_swallowing(false);
                unclip_cursor();
                injector.release_all();
                state_machine.process_event(StateEvent::Disconnected);
                discovery.set_connected(false);

                let st = {
                    let cfg = config.lock().unwrap();
                    if cfg.peer_ip.is_empty() {
                        PeerStatus::SearchingOnLan { port: cfg.peer_port }
                    } else {
                        PeerStatus::WaitingForPeer {
                            local_ip: get_local_ip(),
                            port: cfg.peer_port,
                        }
                    }
                };
                *status.lock().unwrap() = st.clone();
                let _ = status_tx.send(st);
            }

            Err(_) => {
                // Timeout on event loop, continue checking running flag
            }
        }
    }

    // Cleanup on exit
    discovery.stop();
    capture.set_swallowing(false);
    unclip_cursor();
    injector.release_all();
}

async fn run_session_worker(
    mut session: QuicSession,
    out_rx: Receiver<(Packet, bool)>,
    event_tx: Sender<NodeEvent>,
    running: Arc<AtomicBool>,
) {
    let mut interval = tokio::time::interval(Duration::from_millis(1000));

    loop {
        if !running.load(Ordering::SeqCst) {
            break;
        }

        tokio::select! {
            // Read incoming packet from QUIC session
            pkt_res = session.read_next_packet() => {
                match pkt_res {
                    Ok(pkt) => {
                        if event_tx.send(NodeEvent::Network(pkt)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = event_tx.send(NodeEvent::NetworkDisconnected);
                        break;
                    }
                }
            }

            // Write queued outgoing packets to QUIC session
            out_item = async {
                tokio::task::spawn_blocking({
                    let out_rx = out_rx.clone();
                    move || out_rx.recv_timeout(Duration::from_millis(100))
                }).await.unwrap_or(Err(crossbeam_channel::RecvTimeoutError::Timeout))
            } => {
                if let Ok((pkt, unreliable)) = out_item {
                    let res = if unreliable {
                        session.send_unreliable(&pkt)
                    } else {
                        session.send_reliable(&pkt).await
                    };
                    if res.is_err() {
                        let _ = event_tx.send(NodeEvent::NetworkDisconnected);
                        break;
                    }
                }
            }

            // 1-second keepalive & latency check
            _ = interval.tick() => {
                let rtt_ms = session.rtt().as_secs_f32() * 1000.0;
                let _ = event_tx.send(NodeEvent::RttUpdated(rtt_ms));

                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64;

                if session.send_reliable(&Packet::Heartbeat { timestamp_ms: now_ms }).await.is_err() {
                    let _ = event_tx.send(NodeEvent::NetworkDisconnected);
                    break;
                }
            }
        }
    }
}

pub fn get_local_ip() -> String {
    if let Ok(socket) = std::net::UdpSocket::bind("0.0.0.0:0") {
        if socket.connect("8.8.8.8:80").is_ok() {
            if let Ok(addr) = socket.local_addr() {
                return addr.ip().to_string();
            }
        }
    }
    "127.0.0.1".to_string()
}
