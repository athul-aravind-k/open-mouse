use crate::key_tracker::KeyTracker;
use proto::MouseBtn;
use std::mem::size_of;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT,
    KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE,
    MOUSEEVENTF_HWHEEL, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP, MOUSEEVENTF_MIDDLEDOWN,
    MOUSEEVENTF_MIDDLEUP, MOUSEEVENTF_MOVE, MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP,
    MOUSEEVENTF_WHEEL, MOUSEEVENTF_XDOWN, MOUSEEVENTF_XUP, MOUSEINPUT,
    VIRTUAL_KEY,
};

pub struct InputInjector {
    tracker: KeyTracker,
}

impl Default for InputInjector {
    fn default() -> Self {
        Self::new()
    }
}

impl InputInjector {
    pub fn new() -> Self {
        Self {
            tracker: KeyTracker::new(),
        }
    }

    pub fn tracker(&self) -> &KeyTracker {
        &self.tracker
    }

    pub fn move_rel(&self, dx: i32, dy: i32) {
        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: 0,
                    dwFlags: MOUSEEVENTF_MOVE,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_inputs(&[input]);
    }

    pub fn mouse_button(&mut self, button: MouseBtn, pressed: bool) {
        let (flags, mouse_data) = match (button, pressed) {
            (MouseBtn::Left, true) => (MOUSEEVENTF_LEFTDOWN, 0),
            (MouseBtn::Left, false) => (MOUSEEVENTF_LEFTUP, 0),
            (MouseBtn::Right, true) => (MOUSEEVENTF_RIGHTDOWN, 0),
            (MouseBtn::Right, false) => (MOUSEEVENTF_RIGHTUP, 0),
            (MouseBtn::Middle, true) => (MOUSEEVENTF_MIDDLEDOWN, 0),
            (MouseBtn::Middle, false) => (MOUSEEVENTF_MIDDLEUP, 0),
            (MouseBtn::X1, true) => (MOUSEEVENTF_XDOWN, 0x0001),
            (MouseBtn::X1, false) => (MOUSEEVENTF_XUP, 0x0001),
            (MouseBtn::X2, true) => (MOUSEEVENTF_XDOWN, 0x0002),
            (MouseBtn::X2, false) => (MOUSEEVENTF_XUP, 0x0002),
        };

        if pressed {
            self.tracker.button_down(button);
        } else {
            self.tracker.button_up(button);
        }

        let input = INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx: 0,
                    dy: 0,
                    mouseData: mouse_data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_inputs(&[input]);
    }

    pub fn mouse_scroll(&self, delta_x: i32, delta_y: i32) {
        let mut inputs = Vec::with_capacity(2);

        if delta_y != 0 {
            inputs.push(INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: delta_y as u32,
                        dwFlags: MOUSEEVENTF_WHEEL,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }

        if delta_x != 0 {
            inputs.push(INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: delta_x as u32,
                        dwFlags: MOUSEEVENTF_HWHEEL,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }

        if !inputs.is_empty() {
            send_inputs(&inputs);
        }
    }

    pub fn key(&mut self, scancode: u16, extended: bool, pressed: bool) {
        let mut flags = KEYEVENTF_SCANCODE;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if !pressed {
            flags |= KEYEVENTF_KEYUP;
            self.tracker.key_up(scancode);
        } else {
            self.tracker.key_down(scancode);
        }

        let input = INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: VIRTUAL_KEY(0),
                    wScan: scancode,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        };
        send_inputs(&[input]);
    }

    /// Releases all tracked pressed keys and buttons to eliminate stuck keys.
    pub fn release_all(&mut self) {
        let mut inputs = Vec::new();

        // Release all pressed mouse buttons
        for &btn in self.tracker.pressed_buttons() {
            let (flags, mouse_data) = match btn {
                MouseBtn::Left => (MOUSEEVENTF_LEFTUP, 0),
                MouseBtn::Right => (MOUSEEVENTF_RIGHTUP, 0),
                MouseBtn::Middle => (MOUSEEVENTF_MIDDLEUP, 0),
                MouseBtn::X1 => (MOUSEEVENTF_XUP, 0x0001),
                MouseBtn::X2 => (MOUSEEVENTF_XUP, 0x0002),
            };
            inputs.push(INPUT {
                r#type: INPUT_MOUSE,
                Anonymous: INPUT_0 {
                    mi: MOUSEINPUT {
                        dx: 0,
                        dy: 0,
                        mouseData: mouse_data,
                        dwFlags: flags,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }

        // Release all pressed keyboard scancodes
        for &scancode in self.tracker.pressed_keys() {
            inputs.push(INPUT {
                r#type: INPUT_KEYBOARD,
                Anonymous: INPUT_0 {
                    ki: KEYBDINPUT {
                        wVk: VIRTUAL_KEY(0),
                        wScan: scancode,
                        dwFlags: KEYEVENTF_SCANCODE | KEYEVENTF_KEYUP,
                        time: 0,
                        dwExtraInfo: 0,
                    },
                },
            });
        }

        if !inputs.is_empty() {
            send_inputs(&inputs);
        }

        self.tracker.clear();
    }
}

fn send_inputs(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, size_of::<INPUT>() as i32);
    }
}
