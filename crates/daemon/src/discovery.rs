use crossbeam_channel::Receiver;
use proto::{DiscoveryBeacon, DISCOVERY_PORT};
use std::net::{IpAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredPeer {
    pub ip: IpAddr,
    pub port: u16,
    pub machine_id: String,
    pub device_name: String,
}

pub struct DiscoveryService {
    running: Arc<AtomicBool>,
    is_connected: Arc<AtomicBool>,
    peer_rx: Receiver<DiscoveredPeer>,
}

impl DiscoveryService {
    pub fn new(
        local_machine_id: String,
        local_device_name: String,
        local_tcp_port: u16,
    ) -> Self {
        let (peer_tx, peer_rx) = crossbeam_channel::unbounded();
        let running = Arc::new(AtomicBool::new(true));
        let is_connected = Arc::new(AtomicBool::new(false));

        // 1. Broadcaster Thread
        let running_bcast = running.clone();
        let connected_bcast = is_connected.clone();
        let id_bcast = local_machine_id.clone();
        let name_bcast = local_device_name.clone();

        thread::spawn(move || {
            let socket = match UdpSocket::bind("0.0.0.0:0") {
                Ok(s) => {
                    let _ = s.set_broadcast(true);
                    s
                }
                Err(e) => {
                    eprintln!("[Discovery] Failed to bind broadcast socket: {}", e);
                    return;
                }
            };

            let target_addr = format!("255.255.255.255:{}", DISCOVERY_PORT);
            let beacon = DiscoveryBeacon::new(id_bcast, name_bcast, local_tcp_port);
            let beacon_bytes = match beacon.to_bytes() {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("[Discovery] Failed to serialize beacon: {}", e);
                    return;
                }
            };

            while running_bcast.load(Ordering::SeqCst) {
                if !connected_bcast.load(Ordering::SeqCst) {
                    let _ = socket.send_to(&beacon_bytes, &target_addr);
                }
                thread::sleep(Duration::from_millis(2000));
            }
        });

        // 2. Listener Thread
        let running_listener = running.clone();
        let connected_listener = is_connected.clone();
        let id_listener = local_machine_id;

        thread::spawn(move || {
            let socket = match UdpSocket::bind(format!("0.0.0.0:{}", DISCOVERY_PORT)) {
                Ok(s) => {
                    let _ = s.set_broadcast(true);
                    let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                    s
                }
                Err(e) => {
                    eprintln!("[Discovery] Failed to bind listener on port {}: {}", DISCOVERY_PORT, e);
                    return;
                }
            };

            let mut buf = [0u8; 512];

            while running_listener.load(Ordering::SeqCst) {
                match socket.recv_from(&mut buf) {
                    Ok((n, src_addr)) => {
                        // Skip discovery if already connected
                        if connected_listener.load(Ordering::SeqCst) {
                            continue;
                        }

                        if let Ok(beacon) = DiscoveryBeacon::from_bytes(&buf[..n]) {
                            // Ignore self-broadcasts
                            if beacon.machine_id == id_listener {
                                continue;
                            }

                            let discovered = DiscoveredPeer {
                                ip: src_addr.ip(),
                                port: beacon.tcp_port,
                                machine_id: beacon.machine_id,
                                device_name: beacon.device_name,
                            };

                            let _ = peer_tx.send(discovered);
                        }
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock || e.kind() == std::io::ErrorKind::TimedOut => {
                        // Timeout, loop back to check running flag
                    }
                    Err(_) => {
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        });

        Self {
            running,
            is_connected,
            peer_rx,
        }
    }

    pub fn peer_receiver(&self) -> Receiver<DiscoveredPeer> {
        self.peer_rx.clone()
    }

    pub fn set_connected(&self, connected: bool) {
        self.is_connected.store(connected, Ordering::SeqCst);
    }

    pub fn stop(&self) {
        self.running.store(false, Ordering::SeqCst);
    }
}

impl Drop for DiscoveryService {
    fn drop(&mut self) {
        self.stop();
    }
}

/// Deterministic tie-breaker: returns true if local machine should be the active connector.
pub fn should_initiate_connection(local_id: &str, remote_id: &str) -> bool {
    local_id < remote_id
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tie_breaker_symmetry() {
        let laptop_a = "Laptop-A-uuid-1";
        let laptop_b = "Laptop-B-uuid-2";

        // Exactly one should be true, the other false
        let a_connects = should_initiate_connection(laptop_a, laptop_b);
        let b_connects = should_initiate_connection(laptop_b, laptop_a);

        assert!(a_connects != b_connects);
        assert!(a_connects); // "Laptop-A..." < "Laptop-B..."
        assert!(!b_connects);
    }
}
