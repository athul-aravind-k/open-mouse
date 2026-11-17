pub mod edge;
pub mod state;
pub mod config;
pub mod peer;
pub mod discovery;

pub use config::{AppConfig, get_config_path, load_config, save_config};
pub use edge::{DisplayArrangement, EdgeDetector, EdgeDirection, ReturnDetector};
pub use state::{DaemonState, KvmStateMachine, StateAction, StateEvent};
pub use peer::{PeerNode, PeerStatus};
pub use discovery::{DiscoveredPeer, DiscoveryService, should_initiate_connection};

/// Configuration for KVM edge detection and transition timings.
#[derive(Debug, Clone)]
pub struct KvmConfig {
    pub edge_trigger_margin: i32,
    pub return_inward_offset: i32,
    pub cooldown_ms: u64,
    pub entry_lockout_ms: u64,
}

impl Default for KvmConfig {
    fn default() -> Self {
        Self {
            edge_trigger_margin: 2,
            return_inward_offset: 10,
            cooldown_ms: 300,
            entry_lockout_ms: 150,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn test_full_bidirectional_handoff_lifecycle() {
        // Setup Host (1920x1080) and Client (2560x1440) state machines
        let mut host = KvmStateMachine::new(1920, 1080);
        let mut client = KvmStateMachine::new(2560, 1440);

        // 1. Host cursor reaches right edge (x = 1919)
        let host_action = host.process_event(StateEvent::EdgeHit { x: 1919, y: 540 });
        match host_action {
            StateAction::EnterRemote { park_x, park_y, x_ratio, y_ratio } => {
                assert_eq!(park_x, 1919);
                assert_eq!(park_y, 540);
                assert_eq!(x_ratio, 0.0);
                assert!((y_ratio - 0.5).abs() < 1e-4);

                // 2. Client receives Enter packet
                let client_action = client.process_event(StateEvent::RemoteEnterReceived { x_ratio, y_ratio });
                match client_action {
                    StateAction::AcceptRemoteControl { target_x, target_y } => {
                        assert_eq!(target_x, 1);
                        assert_eq!(target_y, 720); // 0.5 * 1440
                    }
                    other => panic!("Expected AcceptRemoteControl, got {:?}", other),
                }
            }
            other => panic!("Expected EnterRemote, got {:?}", other),
        }

        assert!(host.is_remote_active());
        assert!(client.is_remote_controlled());

        // 3. Client user moves cursor back to left boundary after residence
        let zero_lockout_client = client.with_timing(2, 10, Duration::from_millis(300), Duration::from_millis(0));
        let mut client = zero_lockout_client;

        let client_return_action = client.process_event(StateEvent::ReturnBoundaryHit {
            x_ratio: 0.0,
            y_ratio: 0.5,
        });
        match client_return_action {
            StateAction::SendReturnToPeer { x_ratio, y_ratio } => {
                // 4. Host receives Leave packet
                let host_return_action = host.process_event(StateEvent::RemoteLeaveReceived { x_ratio, y_ratio });
                match host_return_action {
                    StateAction::ReturnToLocal { return_x, return_y } => {
                        assert_eq!(return_x, 1910); // 1920 - 10
                        assert_eq!(return_y, 540);
                    }
                    other => panic!("Expected ReturnToLocal, got {:?}", other),
                }
            }
            other => panic!("Expected SendReturnToPeer, got {:?}", other),
        }

        // Both machines back in Local state
        assert!(host.is_local());
        assert!(client.is_local());
        assert!(host.is_in_cooldown());
    }
}
