use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

/// Physical screen edge direction.
#[derive(Serialize, Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeDirection {
    Left,
    Right,
    Top,
    Bottom,
}

/// 2D arrangement of the remote display relative to the local primary display.
#[derive(Serialize, Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct DisplayArrangement {
    pub relative_edge: EdgeDirection,
    pub offset_x: i32,
    pub offset_y: i32,
}

impl Default for DisplayArrangement {
    fn default() -> Self {
        Self {
            relative_edge: EdgeDirection::Right,
            offset_x: 0,
            offset_y: 0,
        }
    }
}

impl DisplayArrangement {
    pub fn opposite_edge(&self) -> EdgeDirection {
        match self.relative_edge {
            EdgeDirection::Left => EdgeDirection::Right,
            EdgeDirection::Right => EdgeDirection::Left,
            EdgeDirection::Top => EdgeDirection::Bottom,
            EdgeDirection::Bottom => EdgeDirection::Top,
        }
    }

    /// Evaluates if a remotely controlled cursor has reached the return boundary
    /// back towards the controlling machine.
    pub fn check_return_boundary(
        &self,
        cur_x: i32,
        cur_y: i32,
        screen_w: i32,
        screen_h: i32,
    ) -> Option<(f32, f32)> {
        match self.relative_edge {
            EdgeDirection::Right => {
                if cur_x <= 0 {
                    let y_ratio = (cur_y as f32 / screen_h as f32).clamp(0.0, 1.0);
                    Some((0.0, y_ratio))
                } else {
                    None
                }
            }
            EdgeDirection::Left => {
                if cur_x >= screen_w - 1 {
                    let y_ratio = (cur_y as f32 / screen_h as f32).clamp(0.0, 1.0);
                    Some((1.0, y_ratio))
                } else {
                    None
                }
            }
            EdgeDirection::Top => {
                if cur_y >= screen_h - 1 {
                    let x_ratio = (cur_x as f32 / screen_w as f32).clamp(0.0, 1.0);
                    Some((x_ratio, 1.0))
                } else {
                    None
                }
            }
            EdgeDirection::Bottom => {
                if cur_y <= 0 {
                    let x_ratio = (cur_x as f32 / screen_w as f32).clamp(0.0, 1.0);
                    Some((x_ratio, 0.0))
                } else {
                    None
                }
            }
        }
    }
}

/// Detects when the cursor reaches a boundary threshold on the host machine,
/// applying hysteresis margin and cooldown lockout.
#[derive(Debug, Clone)]
pub struct EdgeDetector {
    screen_w: i32,
    screen_h: i32,
    margin_px: i32,
    inward_offset_px: i32,
    cooldown_duration: Duration,
    last_return_time: Option<Instant>,
}

impl EdgeDetector {
    /// Creates an edge detector for the specified screen dimensions with default parameters.
    pub fn new(screen_w: i32, screen_h: i32) -> Self {
        Self {
            screen_w,
            screen_h,
            margin_px: 2,
            inward_offset_px: 10,
            cooldown_duration: Duration::from_millis(300),
            last_return_time: None,
        }
    }

    /// Sets custom margin, inward return offset, and cooldown duration.
    pub fn with_config(
        mut self,
        margin_px: i32,
        inward_offset_px: i32,
        cooldown: Duration,
    ) -> Self {
        self.margin_px = margin_px;
        self.inward_offset_px = inward_offset_px;
        self.cooldown_duration = cooldown;
        self
    }

    /// Checks if the cursor is currently in the return cooldown window.
    pub fn is_in_cooldown(&self) -> bool {
        if let Some(last) = self.last_return_time {
            last.elapsed() < self.cooldown_duration
        } else {
            false
        }
    }

    /// Records that the cursor has returned from the remote peer, starting the cooldown timer.
    pub fn record_return(&mut self) {
        self.last_return_time = Some(Instant::now());
    }

    /// Checks if a cursor coordinate has crossed into the trigger zone for an edge.
    ///
    /// For Right edge: triggers when `x >= screen_w - margin_px` and not in cooldown.
    pub fn check_right_edge(&self, x: i32, _y: i32) -> bool {
        if self.is_in_cooldown() {
            return false;
        }
        x >= self.screen_w - self.margin_px
    }

    /// Calculates the parking coordinate for clamping at the right border.
    pub fn calculate_park_position(&self, y: i32) -> (i32, i32) {
        let park_x = self.screen_w - 1;
        let park_y = y.clamp(0, (self.screen_h - 1).max(0));
        (park_x, park_y)
    }

    /// Calculates the safe inward position to restore the cursor when returning from remote screen.
    pub fn calculate_return_position(&self, y_ratio: f32) -> (i32, i32) {
        let return_x = (self.screen_w - self.inward_offset_px).max(0);
        let return_y = (y_ratio * self.screen_h as f32).clamp(0.0, (self.screen_h - 1).max(0) as f32) as i32;
        (return_x, return_y)
    }

    /// Normalizes vertical coordinate into [0.0, 1.0] ratio.
    pub fn normalize_y_ratio(&self, y: i32) -> f32 {
        if self.screen_h <= 0 {
            return 0.0;
        }
        (y as f32 / self.screen_h as f32).clamp(0.0, 1.0)
    }
}

/// Detects when a remote-controlled cursor pushes against the entry border on the client machine
/// to return control to the host, enforcing an initial residence lockout duration.
#[derive(Debug, Clone)]
pub struct ReturnDetector {
    screen_w: i32,
    screen_h: i32,
    entry_lockout_duration: Duration,
    entry_time: Option<Instant>,
    return_threshold_x: i32,
}

impl ReturnDetector {
    /// Creates a return detector for the client screen.
    pub fn new(screen_w: i32, screen_h: i32) -> Self {
        Self {
            screen_w,
            screen_h,
            entry_lockout_duration: Duration::from_millis(150),
            entry_time: None,
            return_threshold_x: 1,
        }
    }

    /// Customizes lockout duration and boundary pixel threshold.
    pub fn with_lockout(mut self, lockout: Duration, return_threshold_x: i32) -> Self {
        self.entry_lockout_duration = lockout;
        self.return_threshold_x = return_threshold_x;
        self
    }

    /// Returns the screen dimensions managed by this detector.
    pub fn screen_size(&self) -> (i32, i32) {
        (self.screen_w, self.screen_h)
    }

    /// Records that the cursor has entered the client screen, beginning the entry lockout.
    pub fn record_entry(&mut self) {
        self.entry_time = Some(Instant::now());
    }

    /// Checks if entry residence lockout is still active.
    pub fn is_in_lockout(&self) -> bool {
        if let Some(entry) = self.entry_time {
            entry.elapsed() < self.entry_lockout_duration
        } else {
            false
        }
    }

    /// Evaluates if the cursor is attempting to cross back off the left edge towards host.
    ///
    /// Triggers if `(cur_x <= return_threshold_x || cur_x + dx <= 0) && dx < 0` and entry lockout expired.
    /// Returns `Some((x_ratio, y_ratio))` if return boundary was triggered.
    pub fn check_return(&self, cur_x: i32, cur_y: i32, dx: i32) -> Option<(f32, f32)> {
        if self.is_in_lockout() {
            return None;
        }

        if (cur_x <= self.return_threshold_x || cur_x + dx <= 0) && dx < 0 {
            let y_ratio = if self.screen_h > 0 {
                (cur_y as f32 / self.screen_h as f32).clamp(0.0, 1.0)
            } else {
                0.0
            };
            Some((0.0, y_ratio))
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_edge_detector_margins_and_cooldown() {
        let mut detector = EdgeDetector::new(1920, 1080);

        // Before edge
        assert!(!detector.check_right_edge(1917, 500));

        // At edge
        assert!(detector.check_right_edge(1918, 500));
        assert!(detector.check_right_edge(1919, 500));

        // Park position
        assert_eq!(detector.calculate_park_position(500), (1919, 500));

        // Return position inward
        assert_eq!(detector.calculate_return_position(0.5), (1910, 540));

        // Record return -> triggers cooldown
        detector.record_return();
        assert!(detector.is_in_cooldown());
        assert!(!detector.check_right_edge(1919, 500));
    }

    #[test]
    fn test_return_detector_lockout_and_trigger() {
        let mut return_det = ReturnDetector::new(1920, 1080);
        return_det.record_entry();

        // While locked out, left edge push is ignored
        assert_eq!(return_det.check_return(0, 500, -5), None);

        // Zero out lockout for testing
        let immediate_det = return_det.with_lockout(Duration::from_millis(0), 1);

        // Rightwards movement does not return
        assert_eq!(immediate_det.check_return(0, 500, 10), None);

        // Leftwards movement at left boundary triggers return
        let hit = immediate_det.check_return(1, 540, -2);
        assert!(hit.is_some());
        let (x_ratio, y_ratio) = hit.unwrap();
        assert_eq!(x_ratio, 0.0);
        assert!((y_ratio - 0.5).abs() < 1e-4);
    }

    #[test]
    fn test_display_arrangement_return_boundaries() {
        let right_arr = DisplayArrangement {
            relative_edge: EdgeDirection::Right,
            offset_x: 0,
            offset_y: 100,
        };
        // When controlled by a machine on the Right, our return edge is Left (x <= 0)
        assert_eq!(right_arr.check_return_boundary(5, 500, 1920, 1080), None);
        assert!(right_arr.check_return_boundary(0, 540, 1920, 1080).is_some());

        let top_arr = DisplayArrangement {
            relative_edge: EdgeDirection::Top,
            offset_x: 50,
            offset_y: 0,
        };
        // When controlled by a machine on Top, our return edge is Bottom (y >= 1079)
        assert_eq!(top_arr.check_return_boundary(500, 500, 1920, 1080), None);
        assert!(top_arr.check_return_boundary(960, 1079, 1920, 1080).is_some());
    }
}
