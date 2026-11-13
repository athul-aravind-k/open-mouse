use std::collections::HashSet;
use proto::MouseBtn;

#[derive(Default, Debug)]
pub struct KeyTracker {
    pressed_keys: HashSet<u16>,
    pressed_buttons: HashSet<MouseBtn>,
}

impl KeyTracker {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn key_down(&mut self, scancode: u16) {
        self.pressed_keys.insert(scancode);
    }

    pub fn key_up(&mut self, scancode: u16) {
        self.pressed_keys.remove(&scancode);
    }

    pub fn button_down(&mut self, button: MouseBtn) {
        self.pressed_buttons.insert(button);
    }

    pub fn button_up(&mut self, button: MouseBtn) {
        self.pressed_buttons.remove(&button);
    }

    pub fn pressed_keys(&self) -> &HashSet<u16> {
        &self.pressed_keys
    }

    pub fn pressed_buttons(&self) -> &HashSet<MouseBtn> {
        &self.pressed_buttons
    }

    pub fn clear(&mut self) {
        self.pressed_keys.clear();
        self.pressed_buttons.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_key_tracker_state() {
        let mut tracker = KeyTracker::new();
        tracker.key_down(0x1E); // A
        tracker.key_down(0x2A); // Left Shift
        tracker.button_down(MouseBtn::Left);

        assert_eq!(tracker.pressed_keys().len(), 2);
        assert_eq!(tracker.pressed_buttons().len(), 1);

        tracker.key_up(0x1E);
        assert_eq!(tracker.pressed_keys().len(), 1);
        assert!(tracker.pressed_keys().contains(&0x2A));

        tracker.clear();
        assert!(tracker.pressed_keys().is_empty());
        assert!(tracker.pressed_buttons().is_empty());
    }
}
