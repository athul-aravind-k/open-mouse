use crate::config::AppConfig;
use crate::discovery::{DiscoveryService, should_initiate_connection};
use crate::edge::EdgeDirection;
use crate::state::{KvmStateMachine, StateAction, StateEvent};
use capture::{
    get_cursor_pos, get_primary_screen_size, park_cursor, set_cursor_pos, unclip_cursor,
    CaptureEngine, CapturedEvent,
};
use crossbeam_channel::{Receiver, Sender};
use emulate::InputInjector;
use proto::{read_packet_framed, write_packet_framed, Packet, ScreenDescriptor};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

#[derive(Debug, Clone, PartialEq)]
pub enum PeerStatus {
    SearchingOnLan { port: u16 },
    WaitingForPeer { local_ip: String, port: u16 },
    Connecting { target: String },
    Connected { peer_name: String, peer_addr: String },
    ControllingRemote,
    RemoteControlled,
}

pub struct PeerNode {
    config: Arc<Mutex<AppConfig>>,
    status: Arc<Mutex<PeerStatus>>,
    status_rx: Receiver<PeerStatus>,
    running: Arc<AtomicBool>,
}

impl PeerNode {
    pub fn new(initial_config: AppConfig) -> Self {
        let (status_tx, status_rx) = crossbeam_channel::unbounded();
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

        thread::spawn(move || {
            run_peer_lifecycle(config_clone, status_clone, status_tx, running_clone);
        });

        Self {
            config,
            status,
            status_rx,
            running,
        }
    }

    pub fn get_status(&self) -> PeerStatus {
        self.status.lock().unwrap().clone()
    }

    pub fn status_receiver(&self) -> Receiver<PeerStatus> {
        self.status_rx.clone()
    }

    pub fn update_peer_ip(&self, new_ip: String) {
        let mut cfg = self.config.lock().unwrap();
        cfg.peer_ip = new_ip;
        let _ = crate::config::save_config(&cfg);
    }

    pub fn toggle_edge(&self) -> EdgeDirection {
        let mut cfg = self.config.lock().unwrap();
        let new_edge = match cfg.shared_edge {
            EdgeDirection::Right => EdgeDirection::Left,
            EdgeDirection::Left => EdgeDirection::Right,
            _ => EdgeDirection::Right,
        };
        cfg.shared_edge = new_edge;
        let _ = crate::config::save_config(&cfg);
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
    NewConnection(TcpStream, String),
}

fn run_peer_lifecycle(
    config: Arc<Mutex<AppConfig>>,
    status: Arc<Mutex<PeerStatus>>,
    status_tx: Sender<PeerStatus>,
    running: Arc<AtomicBool>,
) {
    let (machine_id, device_name, port) = {
        let cfg = config.lock().unwrap();
        (cfg.machine_id.clone(), cfg.device_name.clone(), cfg.peer_port)
    };

    let bind_addr = format!("0.0.0.0:{}", port);
    let listener = match TcpListener::bind(&bind_addr) {
        Ok(l) => {
            l.set_nonblocking(true).ok();
            Some(l)
        }
        Err(e) => {
            eprintln!("[PeerNode] Failed to bind listener on {}: {}", bind_addr, e);
            None
        }
    };

    // Initialize UDP LAN Auto-Discovery
    let discovery = Arc::new(DiscoveryService::new(
        machine_id.clone(),
        device_name.clone(),
        port,
    ));

    let (event_tx, event_rx) = crossbeam_channel::unbounded::<NodeEvent>();

    // 1. Thread for accepting incoming connections
    if let Some(listener) = listener {
        let event_tx_inbound = event_tx.clone();
        let running_inbound = running.clone();
        thread::spawn(move || {
            while running_inbound.load(Ordering::SeqCst) {
                match listener.accept() {
                    Ok((stream, peer_addr)) => {
                        stream.set_nodelay(true).ok();
                        let _ = event_tx_inbound.send(NodeEvent::NewConnection(stream, peer_addr.to_string()));
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(100));
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(500));
                    }
                }
            }
        });
    }

    // 2. Thread for outbound connection attempts (Manual IP or UDP Auto-Discovered)
    let config_outbound = config.clone();
    let event_tx_outbound = event_tx.clone();
    let running_outbound = running.clone();
    let discovery_peer_rx = discovery.peer_receiver();
    let local_id = machine_id.clone();

    thread::spawn(move || {
        while running_outbound.load(Ordering::SeqCst) {
            let (manual_ip, port) = {
                let cfg = config_outbound.lock().unwrap();
                (cfg.peer_ip.clone(), cfg.peer_port)
            };

            if !manual_ip.is_empty() && manual_ip != "127.0.0.1" && manual_ip != "0.0.0.0" {
                // Manual IP specified in config
                let target = format!("{}:{}", manual_ip, port);
                if let Ok(stream) = TcpStream::connect(&target) {
                    stream.set_nodelay(true).ok();
                    let _ = event_tx_outbound.send(NodeEvent::NewConnection(stream, target));
                    thread::sleep(Duration::from_secs(5));
                }
                thread::sleep(Duration::from_secs(3));
            } else {
                // Auto-discovery mode: process discovered beacons
                match discovery_peer_rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(peer) => {
                        // Tie-breaker: only connect if our machine ID is smaller
                        if should_initiate_connection(&local_id, &peer.machine_id) {
                            let target = format!("{}:{}", peer.ip, peer.port);
                            if let Ok(stream) = TcpStream::connect(&target) {
                                stream.set_nodelay(true).ok();
                                let _ = event_tx_outbound.send(NodeEvent::NewConnection(stream, target));
                                thread::sleep(Duration::from_secs(5));
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

    let (screen_w, screen_h) = get_primary_screen_size();
    let mut state_machine = KvmStateMachine::new(screen_w, screen_h);
    let mut injector = InputInjector::new();

    let mut active_write_stream: Option<TcpStream> = None;
    let mut current_peer_name = String::new();
    let mut is_controlled = false;
    let mut entry_time = Instant::now();
    let entry_lockout = Duration::from_millis(150);

    while running.load(Ordering::SeqCst) {
        match event_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(NodeEvent::NewConnection(stream, addr_str)) => {
                // If we don't already have an active stream, adopt this one
                if active_write_stream.is_none() {
                    let mut write_stream = match stream.try_clone() {
                        Ok(s) => s,
                        Err(_) => continue,
                    };
                    let mut read_stream = stream;

                    // Send Hello packet
                    let device_name = config.lock().unwrap().device_name.clone();
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
                    };
                    let _ = write_packet_framed(&mut write_stream, &hello);

                    active_write_stream = Some(write_stream);
                    current_peer_name = addr_str.clone();
                    state_machine.process_event(StateEvent::Connected);
                    discovery.set_connected(true);

                    let new_status = PeerStatus::Connected {
                        peer_name: addr_str.clone(),
                        peer_addr: addr_str,
                    };
                    *status.lock().unwrap() = new_status.clone();
                    let _ = status_tx.send(new_status);

                    // Background thread: read from this stream
                    let event_tx_reader = event_tx.clone();
                    thread::spawn(move || {
                        loop {
                            match read_packet_framed(&mut read_stream) {
                                Ok(pkt) => {
                                    if event_tx_reader.send(NodeEvent::Network(pkt)).is_err() {
                                        break;
                                    }
                                }
                                Err(_) => {
                                    let _ = event_tx_reader.send(NodeEvent::NetworkDisconnected);
                                    break;
                                }
                            }
                        }
                    });
                }
            }

            Ok(NodeEvent::LocalInput(CapturedEvent::EdgeTrigger { x, y })) => {
                if active_write_stream.is_some() && !state_machine.is_remote_active() {
                    let action = state_machine.process_event(StateEvent::EdgeHit { x, y });
                    if let StateAction::EnterRemote { park_x, park_y, x_ratio, y_ratio } = action {
                        capture.set_swallowing(true);
                        park_cursor(park_x, park_y);

                        if let Some(ref mut s) = active_write_stream {
                            let enter_pkt = Packet::Enter { x_ratio, y_ratio };
                            if write_packet_framed(s, &enter_pkt).is_err() {
                                capture.set_swallowing(false);
                                unclip_cursor();
                                state_machine.process_event(StateEvent::EmergencyResetReceived);
                            } else {
                                let st = PeerStatus::ControllingRemote;
                                *status.lock().unwrap() = st.clone();
                                let _ = status_tx.send(st);
                            }
                        }
                    }
                }
            }

            Ok(NodeEvent::LocalInput(CapturedEvent::Packet(pkt))) => {
                if state_machine.is_remote_active() {
                    if let Some(ref mut s) = active_write_stream {
                        if write_packet_framed(s, &pkt).is_err() {
                            capture.set_swallowing(false);
                            unclip_cursor();
                            state_machine.process_event(StateEvent::EmergencyResetReceived);
                        }
                    }
                }
            }

            Ok(NodeEvent::LocalInput(CapturedEvent::EmergencyEscape)) => {
                capture.set_swallowing(false);
                unclip_cursor();
                set_cursor_pos(screen_w / 2, screen_h / 2);
                injector.release_all();
                state_machine.process_event(StateEvent::EmergencyEscape);

                if let Some(ref mut s) = active_write_stream {
                    let _ = write_packet_framed(s, &Packet::EmergencyReset);
                }

                let st = PeerStatus::Connected {
                    peer_name: current_peer_name.clone(),
                    peer_addr: current_peer_name.clone(),
                };
                *status.lock().unwrap() = st.clone();
                let _ = status_tx.send(st);
            }

            Ok(NodeEvent::Network(pkt)) => {
                match pkt {
                    Packet::Hello { machine_name, .. } => {
                        current_peer_name = machine_name.clone();
                        let st = PeerStatus::Connected {
                            peer_name: machine_name,
                            peer_addr: current_peer_name.clone(),
                        };
                        *status.lock().unwrap() = st.clone();
                        let _ = status_tx.send(st);
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
                                peer_addr: current_peer_name.clone(),
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
                                if cur_x <= 0 {
                                    let y_ratio = (cur_y as f32 / screen_h as f32).clamp(0.0, 1.0);
                                    let action = state_machine.process_event(StateEvent::ReturnBoundaryHit {
                                        x_ratio: 0.0,
                                        y_ratio,
                                    });
                                    if let StateAction::SendReturnToPeer { x_ratio, y_ratio } = action {
                                        is_controlled = false;
                                        injector.release_all();

                                        if let Some(ref mut s) = active_write_stream {
                                            let _ = write_packet_framed(s, &Packet::Leave { x_ratio, y_ratio });
                                        }

                                        let st = PeerStatus::Connected {
                                            peer_name: current_peer_name.clone(),
                                            peer_addr: current_peer_name.clone(),
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
                            peer_addr: current_peer_name.clone(),
                        };
                        *status.lock().unwrap() = st.clone();
                        let _ = status_tx.send(st);
                    }

                    _ => {}
                }
            }

            Ok(NodeEvent::NetworkDisconnected) => {
                active_write_stream = None;
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
