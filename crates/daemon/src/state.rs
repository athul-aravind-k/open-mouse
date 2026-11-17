use crate::edge::{DisplayArrangement, EdgeDirection};
use std::time::{Duration, Instant};

/// Core states of the OpenMouse KVM daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonState {
    /// Local machine in control of its own desktop. Detecting edge crossings.
    Local,
    /// Local cursor is parked; streaming hardware input packets to remote peer.
    RemoteActive,
    /// Remote peer is controlling this machine; injecting remote inputs and monitoring return boundary.
    RemoteControlled,
    /// Disconnected from peer or awaiting network connection.
    Disconnected,
}

impl Default for DaemonState {
    fn default() -> Self {
        Self::Disconnected
    }
}

/// Events that drive state machine transitions.
#[derive(Debug, Clone, PartialEq)]
pub enum StateEvent {
    Connected,
    Disconnected,
    EdgeHit { x: i32, y: i32 },
    RemoteEnterReceived { x_ratio: f32, y_ratio: f32 },
    RemoteLeaveReceived { x_ratio: f32, y_ratio: f32 },
    ReturnBoundaryHit { x_ratio: f32, y_ratio: f32 },
    EmergencyEscape,
    EmergencyResetReceived,
}

/// Actions commanded by state machine transitions.
#[derive(Debug, Clone, PartialEq)]
pub enum StateAction {
    None,
    EnterRemote {
        park_x: i32,
        park_y: i32,
        x_ratio: f32,
        y_ratio: f32,
    },
    ReturnToLocal {
        return_x: i32,
        return_y: i32,
    },
    AcceptRemoteControl {
        target_x: i32,
        target_y: i32,
    },
    SendReturnToPeer {
        x_ratio: f32,
        y_ratio: f32,
    },
    EmergencyRecoverLocal {
        center_x: i32,
        center_y: i32,
    },
}

/// Coordinates KVM state transitions, margins, and emergency recoveries.
#[derive(Debug)]
pub struct KvmStateMachine {
    state: DaemonState,
    screen_w: i32,
    screen_h: i32,
    remote_w: i32,
    remote_h: i32,
    arrangement: DisplayArrangement,
    edge_trigger_margin: i32,
    return_inward_offset: i32,
    cooldown_duration: Duration,
    last_return_instant: Option<Instant>,
    entry_lockout_duration: Duration,
    last_enter_instant: Option<Instant>,
}

impl KvmStateMachine {
    /// Creates a new KVM state machine with screen dimensions and default timing constants.
    pub fn new(screen_w: i32, screen_h: i32) -> Self {
        Self {
            state: DaemonState::Local,
            screen_w,
            screen_h,
            remote_w: screen_w,
            remote_h: screen_h,
            arrangement: DisplayArrangement::default(),
            edge_trigger_margin: 2,
            return_inward_offset: 10,
            cooldown_duration: Duration::from_millis(300),
            last_return_instant: None,
            entry_lockout_duration: Duration::from_millis(150),
            last_enter_instant: None,
        }
    }

    pub fn set_arrangement(&mut self, arrangement: DisplayArrangement) {
        self.arrangement = arrangement;
    }

    pub fn arrangement(&self) -> &DisplayArrangement {
        &self.arrangement
    }

    pub fn set_remote_screen_size(&mut self, w: u32, h: u32) {
        if w > 0 && h > 0 {
            self.remote_w = w as i32;
            self.remote_h = h as i32;
        }
    }

    /// Customizes hysteresis and timing parameters.
    pub fn with_timing(
        mut self,
        edge_trigger_margin: i32,
        return_inward_offset: i32,
        cooldown: Duration,
        entry_lockout: Duration,
    ) -> Self {
        self.edge_trigger_margin = edge_trigger_margin;
        self.return_inward_offset = return_inward_offset;
        self.cooldown_duration = cooldown;
        self.entry_lockout_duration = entry_lockout;
        self
    }

    /// Current operational state.
    pub fn state(&self) -> DaemonState {
        self.state
    }

    pub fn is_local(&self) -> bool {
        self.state == DaemonState::Local
    }

    pub fn is_remote_active(&self) -> bool {
        self.state == DaemonState::RemoteActive
    }

    pub fn is_remote_controlled(&self) -> bool {
        self.state == DaemonState::RemoteControlled
    }

    pub fn is_disconnected(&self) -> bool {
        self.state == DaemonState::Disconnected
    }

    /// Checks whether edge trigger cooldown is currently active.
    pub fn is_in_cooldown(&self) -> bool {
        if let Some(last_return) = self.last_return_instant {
            last_return.elapsed() < self.cooldown_duration
        } else {
            false
        }
    }

    /// Checks whether entry residence lockout is active on the controlled receiver.
    pub fn is_in_entry_lockout(&self) -> bool {
        if let Some(last_enter) = self.last_enter_instant {
            last_enter.elapsed() < self.entry_lockout_duration
        } else {
            false
        }
    }

    /// Processes an input or network event, transitioning state and returning commanded action.
    pub fn process_event(&mut self, event: StateEvent) -> StateAction {
        match (&self.state, event) {
            // Connection events
            (DaemonState::Disconnected, StateEvent::Connected) => {
                self.state = DaemonState::Local;
                StateAction::None
            }
            (_, StateEvent::Disconnected) => {
                let prev = self.state;
                self.state = DaemonState::Disconnected;
                if prev == DaemonState::RemoteActive || prev == DaemonState::RemoteControlled {
                    StateAction::EmergencyRecoverLocal {
                        center_x: self.screen_w / 2,
                        center_y: self.screen_h / 2,
                    }
                } else {
                    StateAction::None
                }
            }

            // Local -> RemoteActive transition (Cursor reaches configured relative edge)
            (DaemonState::Local, StateEvent::EdgeHit { x, y }) => {
                if self.is_in_cooldown() {
                    return StateAction::None;
                }

                match self.arrangement.relative_edge {
                    EdgeDirection::Right => {
                        let crossing = x >= self.screen_w - self.edge_trigger_margin;
                        let in_span = y >= self.arrangement.offset_y
                            && y < self.arrangement.offset_y + self.remote_h;
                        if crossing && in_span {
                            self.state = DaemonState::RemoteActive;
                            let park_x = self.screen_w - 1;
                            let park_y = y.clamp(0, (self.screen_h - 1).max(0));
                            let target_y = y - self.arrangement.offset_y;
                            let y_ratio = (target_y as f32 / self.remote_h as f32).clamp(0.0, 1.0);
                            StateAction::EnterRemote {
                                park_x,
                                park_y,
                                x_ratio: 0.0,
                                y_ratio,
                            }
                        } else {
                            StateAction::None
                        }
                    }
                    EdgeDirection::Left => {
                        let crossing = x <= self.edge_trigger_margin;
                        let in_span = y >= self.arrangement.offset_y
                            && y < self.arrangement.offset_y + self.remote_h;
                        if crossing && in_span {
                            self.state = DaemonState::RemoteActive;
                            let park_x = 0;
                            let park_y = y.clamp(0, (self.screen_h - 1).max(0));
                            let target_y = y - self.arrangement.offset_y;
                            let y_ratio = (target_y as f32 / self.remote_h as f32).clamp(0.0, 1.0);
                            StateAction::EnterRemote {
                                park_x,
                                park_y,
                                x_ratio: 1.0,
                                y_ratio,
                            }
                        } else {
                            StateAction::None
                        }
                    }
                    EdgeDirection::Top => {
                        let crossing = y <= self.edge_trigger_margin;
                        let in_span = x >= self.arrangement.offset_x
                            && x < self.arrangement.offset_x + self.remote_w;
                        if crossing && in_span {
                            self.state = DaemonState::RemoteActive;
                            let park_x = x.clamp(0, (self.screen_w - 1).max(0));
                            let park_y = 0;
                            let target_x = x - self.arrangement.offset_x;
                            let x_ratio = (target_x as f32 / self.remote_w as f32).clamp(0.0, 1.0);
                            StateAction::EnterRemote {
                                park_x,
                                park_y,
                                x_ratio,
                                y_ratio: 1.0,
                            }
                        } else {
                            StateAction::None
                        }
                    }
                    EdgeDirection::Bottom => {
                        let crossing = y >= self.screen_h - self.edge_trigger_margin;
                        let in_span = x >= self.arrangement.offset_x
                            && x < self.arrangement.offset_x + self.remote_w;
                        if crossing && in_span {
                            self.state = DaemonState::RemoteActive;
                            let park_x = x.clamp(0, (self.screen_w - 1).max(0));
                            let park_y = (self.screen_h - 1).max(0);
                            let target_x = x - self.arrangement.offset_x;
                            let x_ratio = (target_x as f32 / self.remote_w as f32).clamp(0.0, 1.0);
                            StateAction::EnterRemote {
                                park_x,
                                park_y,
                                x_ratio,
                                y_ratio: 0.0,
                            }
                        } else {
                            StateAction::None
                        }
                    }
                }
            }

            // RemoteActive -> Local return (Peer returned control via Leave packet)
            (DaemonState::RemoteActive, StateEvent::RemoteLeaveReceived { x_ratio, y_ratio }) => {
                self.state = DaemonState::Local;
                self.last_return_instant = Some(Instant::now());

                let (return_x, return_y) = match self.arrangement.relative_edge {
                    EdgeDirection::Right => {
                        let rx = (self.screen_w - self.return_inward_offset).max(0);
                        let ry = ((y_ratio * self.remote_h as f32) as i32 + self.arrangement.offset_y)
                            .clamp(0, (self.screen_h - 1).max(0));
                        (rx, ry)
                    }
                    EdgeDirection::Left => {
                        let rx = self.return_inward_offset.min((self.screen_w - 1).max(0));
                        let ry = ((y_ratio * self.remote_h as f32) as i32 + self.arrangement.offset_y)
                            .clamp(0, (self.screen_h - 1).max(0));
                        (rx, ry)
                    }
                    EdgeDirection::Top => {
                        let rx = ((x_ratio * self.remote_w as f32) as i32 + self.arrangement.offset_x)
                            .clamp(0, (self.screen_w - 1).max(0));
                        let ry = self.return_inward_offset.min((self.screen_h - 1).max(0));
                        (rx, ry)
                    }
                    EdgeDirection::Bottom => {
                        let rx = ((x_ratio * self.remote_w as f32) as i32 + self.arrangement.offset_x)
                            .clamp(0, (self.screen_w - 1).max(0));
                        let ry = (self.screen_h - self.return_inward_offset).max(0);
                        (rx, ry)
                    }
                };

                StateAction::ReturnToLocal {
                    return_x,
                    return_y,
                }
            }

            // Local -> RemoteControlled transition (Peer sent Enter packet)
            (DaemonState::Local, StateEvent::RemoteEnterReceived { x_ratio, y_ratio }) => {
                self.state = DaemonState::RemoteControlled;
                self.last_enter_instant = Some(Instant::now());
                let target_x = ((x_ratio * self.screen_w as f32) as i32).max(1).min(self.screen_w - 2);
                let target_y = ((y_ratio * self.screen_h as f32) as i32).clamp(0, (self.screen_h - 1).max(0));
                StateAction::AcceptRemoteControl {
                    target_x,
                    target_y,
                }
            }

            // RemoteControlled -> Local return (Controlled cursor crossed back over return edge)
            (DaemonState::RemoteControlled, StateEvent::ReturnBoundaryHit { x_ratio, y_ratio }) => {
                if !self.is_in_entry_lockout() {
                    self.state = DaemonState::Local;
                    StateAction::SendReturnToPeer {
                        x_ratio,
                        y_ratio,
                    }
                } else {
                    StateAction::None
                }
            }

            // Emergency Escape / Reset across any active remote state
            (_, StateEvent::EmergencyEscape) | (_, StateEvent::EmergencyResetReceived) => {
                let prev = self.state;
                self.state = DaemonState::Local;
                if prev == DaemonState::RemoteActive || prev == DaemonState::RemoteControlled {
                    StateAction::EmergencyRecoverLocal {
                        center_x: self.screen_w / 2,
                        center_y: self.screen_h / 2,
                    }
                } else {
                    StateAction::None
                }
            }

            // Unhandled / benign events in current state
            _ => StateAction::None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_initial_state() {
        let sm = KvmStateMachine::new(1920, 1080);
        assert_eq!(sm.state(), DaemonState::Local);
        assert!(sm.is_local());
    }

    #[test]
    fn test_edge_hit_and_hysteresis_cooldown() {
        let mut sm = KvmStateMachine::new(1920, 1080);

        // Edge hit outside margin does not trigger
        let action = sm.process_event(StateEvent::EdgeHit { x: 1917, y: 500 });
        assert_eq!(action, StateAction::None);
        assert_eq!(sm.state(), DaemonState::Local);

        // Edge hit at margin (x >= 1918) triggers EnterRemote
        let action = sm.process_event(StateEvent::EdgeHit { x: 1919, y: 540 });
        assert_eq!(
            action,
            StateAction::EnterRemote {
                park_x: 1919,
                park_y: 540,
                x_ratio: 0.0,
                y_ratio: 0.5,
            }
        );
        assert_eq!(sm.state(), DaemonState::RemoteActive);

        // Return from remote
        let action = sm.process_event(StateEvent::RemoteLeaveReceived {
            x_ratio: 0.0,
            y_ratio: 0.5,
        });
        assert_eq!(
            action,
            StateAction::ReturnToLocal {
                return_x: 1910,
                return_y: 540,
            }
        );
        assert_eq!(sm.state(), DaemonState::Local);
        assert!(sm.is_in_cooldown());

        // During cooldown, edge hit is ignored
        let action = sm.process_event(StateEvent::EdgeHit { x: 1919, y: 540 });
        assert_eq!(action, StateAction::None);
        assert_eq!(sm.state(), DaemonState::Local);
    }

    #[test]
    fn test_remote_controlled_and_entry_lockout() {
        let mut sm = KvmStateMachine::new(1920, 1080);

        // Receive Enter from peer
        let action = sm.process_event(StateEvent::RemoteEnterReceived {
            x_ratio: 0.0,
            y_ratio: 0.25,
        });
        assert_eq!(
            action,
            StateAction::AcceptRemoteControl {
                target_x: 1,
                target_y: 270,
            }
        );
        assert_eq!(sm.state(), DaemonState::RemoteControlled);
        assert!(sm.is_in_entry_lockout());

        // Immediately hitting return boundary within lockout is suppressed
        let action = sm.process_event(StateEvent::ReturnBoundaryHit {
            x_ratio: 0.0,
            y_ratio: 0.25,
        });
        assert_eq!(action, StateAction::None);
        assert_eq!(sm.state(), DaemonState::RemoteControlled);
    }

    #[test]
    fn test_emergency_escape() {
        let mut sm = KvmStateMachine::new(1920, 1080);
        sm.process_event(StateEvent::EdgeHit { x: 1919, y: 500 });
        assert_eq!(sm.state(), DaemonState::RemoteActive);

        let action = sm.process_event(StateEvent::EmergencyEscape);
        assert_eq!(
            action,
            StateAction::EmergencyRecoverLocal {
                center_x: 960,
                center_y: 540,
            }
        );
        assert_eq!(sm.state(), DaemonState::Local);
    }
}
