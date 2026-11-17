use daemon::{load_config, PeerNode, PeerStatus};
use std::time::Duration;

fn main() {
    println!("OpenMouse CLI Test Tool");
    let config = load_config();
    println!("Device: {} (Machine ID: {})", config.device_name, config.machine_id);
    println!("Shared Edge: {:?}", config.shared_edge);
    println!("Peer IP: '{}' (Port: {})", config.peer_ip, config.peer_port);

    let peer = PeerNode::new(config);
    let status_rx = peer.status_receiver();

    println!("PeerNode running. Listening for status changes (Press Ctrl+C to stop)...");
    while let Ok(status) = status_rx.recv_timeout(Duration::from_secs(60)) {
        match status {
            PeerStatus::SearchingOnLan { port } => {
                println!("[Status] Searching on LAN (broadcast port 48898, QUIC port {})", port);
            }
            PeerStatus::WaitingForPeer { local_ip, port } => {
                println!("[Status] Waiting for peer connection on {}:{}", local_ip, port);
            }
            PeerStatus::Connecting { target } => {
                println!("[Status] Connecting to target: {}", target);
            }
            PeerStatus::Connected { peer_name, peer_addr, rtt_ms, .. } => {
                println!("[Status] Connected to {} at {} (RTT: {:?})", peer_name, peer_addr, rtt_ms);
            }
            PeerStatus::ControllingRemote => {
                println!("[Status] Controlling Remote Screen");
            }
            PeerStatus::RemoteControlled => {
                println!("[Status] Remote Controlled by Peer");
            }
        }
    }
}
