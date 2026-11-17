use crossbeam_channel::{Receiver, Sender};
use proto::Packet;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::Win32::Foundation::{LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::UI::Input::KeyboardAndMouse::{VK_CONTROL, VK_ESCAPE, VK_MENU};
use windows::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, DispatchMessageW, GetCursorPos, GetMessageW, PeekMessageW, PostThreadMessageW,
    SetWindowsHookExW, TranslateMessage, UnhookWindowsHookEx, KBDLLHOOKSTRUCT,
    LLKHF_INJECTED, LLMHF_INJECTED, MSLLHOOKSTRUCT, MSG, PM_NOREMOVE, WH_KEYBOARD_LL, WH_MOUSE_LL,
    WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_QUIT, WM_RBUTTONDOWN, WM_RBUTTONUP,
    WM_SYSKEYDOWN, WM_USER,
};

pub const WM_REHOOK: u32 = WM_USER + 201;

#[derive(Debug, Clone)]
pub enum CapturedEvent {
    Packet(Packet),
    EdgeTrigger { x: i32, y: i32 },
    EmergencyEscape,
}

struct HookState {
    sender: Sender<CapturedEvent>,
    swallow: Arc<AtomicBool>,
    last_x: i32,
    last_y: i32,
    ctrl_pressed: bool,
    alt_pressed: bool,
}

static HOOK_STATE: Mutex<Option<HookState>> = Mutex::new(None);

use crate::cursor::unclip_cursor;
use crate::raw_input::RawInputSink;

pub struct CaptureEngine {
    swallow: Arc<AtomicBool>,
    thread_id: u32,
    raw_sink: RawInputSink,
}

impl CaptureEngine {
    pub fn new() -> (Self, Receiver<CapturedEvent>) {
        let (tx, rx) = crossbeam_channel::unbounded();
        let swallow = Arc::new(AtomicBool::new(false));
        let swallow_clone = swallow.clone();

        let raw_sink = RawInputSink::new_with_enabled(tx.clone(), swallow.clone())
            .expect("Failed to initialize RawInputSink");

        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);

        thread::spawn(move || {
            let mut pt = POINT { x: 0, y: 0 };
            unsafe {
                let _ = GetCursorPos(&mut pt);
            }

            {
                let mut state = HOOK_STATE.lock().unwrap();
                *state = Some(HookState {
                    sender: tx,
                    swallow: swallow_clone,
                    last_x: pt.x,
                    last_y: pt.y,
                    ctrl_pressed: false,
                    alt_pressed: false,
                });
            }

            // Force creation of thread message queue before publishing thread_id
            let mut init_msg = MSG::default();
            unsafe {
                let _ = PeekMessageW(&mut init_msg, None, 0, 0, PM_NOREMOVE);
            }

            let mut mouse_hook = unsafe {
                SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).ok()
            };
            let mut kbd_hook = unsafe {
                SetWindowsHookExW(WH_KEYBOARD_LL, Some(kbd_proc), None, 0).ok()
            };

            let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
            ready_tx.send(thread_id).unwrap();

            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    if msg.message == WM_REHOOK {
                        if let Some(h) = mouse_hook.take() {
                            let _ = UnhookWindowsHookEx(h);
                        }
                        if let Some(h) = kbd_hook.take() {
                            let _ = UnhookWindowsHookEx(h);
                        }

                        if let Ok(mut state_guard) = HOOK_STATE.lock() {
                            if let Some(state) = state_guard.as_mut() {
                                state.ctrl_pressed = false;
                                state.alt_pressed = false;
                                let mut cur_pt = POINT { x: 0, y: 0 };
                                if GetCursorPos(&mut cur_pt).is_ok() {
                                    state.last_x = cur_pt.x;
                                    state.last_y = cur_pt.y;
                                }
                            }
                        }

                        mouse_hook = SetWindowsHookExW(WH_MOUSE_LL, Some(mouse_proc), None, 0).ok();
                        kbd_hook = SetWindowsHookExW(WH_KEYBOARD_LL, Some(kbd_proc), None, 0).ok();
                        continue;
                    }

                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                if let Some(h) = mouse_hook {
                    let _ = UnhookWindowsHookEx(h);
                }
                if let Some(h) = kbd_hook {
                    let _ = UnhookWindowsHookEx(h);
                }
            }
        });

        let thread_id = ready_rx.recv().expect("Failed to initialize hook thread");

        (Self { swallow, thread_id, raw_sink }, rx)
    }

    pub fn rehook(&self) -> bool {
        unsafe {
            PostThreadMessageW(self.thread_id, WM_REHOOK, WPARAM(0), LPARAM(0)).is_ok()
        }
    }

    pub fn set_swallowing(&self, swallow: bool) {
        self.swallow.store(swallow, Ordering::SeqCst);
    }

    pub fn is_swallowing(&self) -> bool {
        self.swallow.load(Ordering::SeqCst)
    }

    pub fn raw_sink(&self) -> &RawInputSink {
        &self.raw_sink
    }
}

impl Drop for CaptureEngine {
    fn drop(&mut self) {
        self.set_swallowing(false);
        unclip_cursor();
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn mouse_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if n_code >= 0 {
        let hook_struct = *(l_param.0 as *const MSLLHOOKSTRUCT);

        // Ignore injected events to avoid feedback loops
        if (hook_struct.flags & LLMHF_INJECTED) != 0 {
            return CallNextHookEx(None, n_code, w_param, l_param);
        }

        if let Ok(mut lock) = HOOK_STATE.try_lock() {
            if let Some(state) = lock.as_mut() {
                let swallow = state.swallow.load(Ordering::SeqCst);
                let msg_type = w_param.0 as u32;

                if msg_type == WM_MOUSEMOVE {
                    let cur_x = hook_struct.pt.x;
                    let cur_y = hook_struct.pt.y;
                    state.last_x = cur_x;
                    state.last_y = cur_y;

                    if swallow {
                        // Swallowing is active: cursor is parked at border.
                        // Swallow local mouse movement (RawInputSink supplies relative mickeys).
                        return LRESULT(1);
                    } else {
                        let _ = state.sender.send(CapturedEvent::EdgeTrigger { x: cur_x, y: cur_y });
                    }
                } else if swallow {
                    // When swallowing is active, suppress local mouse buttons and scroll
                    // so local desktop windows do not receive accidental clicks.
                    // RawInputSink emits the corresponding network packets.
                    match msg_type {
                        WM_LBUTTONDOWN | WM_LBUTTONUP | WM_RBUTTONDOWN | WM_RBUTTONUP
                        | WM_MBUTTONDOWN | WM_MBUTTONUP | WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                            return LRESULT(1);
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    CallNextHookEx(None, n_code, w_param, l_param)
}

unsafe extern "system" fn kbd_proc(n_code: i32, w_param: WPARAM, l_param: LPARAM) -> LRESULT {
    if n_code >= 0 {
        let hook_struct = *(l_param.0 as *const KBDLLHOOKSTRUCT);

        // Ignore injected events
        if (hook_struct.flags.0 & LLKHF_INJECTED.0) != 0 {
            return CallNextHookEx(None, n_code, w_param, l_param);
        }

        let msg_type = w_param.0 as u32;
        let is_down = msg_type == WM_KEYDOWN || msg_type == WM_SYSKEYDOWN;
        let vk = hook_struct.vkCode;

        if let Ok(mut lock) = HOOK_STATE.try_lock() {
            if let Some(state) = lock.as_mut() {
                // Track modifier states for emergency hotkey (Ctrl + Alt + Esc)
                if vk == VK_CONTROL.0 as u32 {
                    state.ctrl_pressed = is_down;
                } else if vk == VK_MENU.0 as u32 {
                    state.alt_pressed = is_down;
                }

                // Check emergency escape condition: Ctrl + Alt + Esc pressed
                if is_down && vk == VK_ESCAPE.0 as u32 && state.ctrl_pressed && state.alt_pressed {
                    state.swallow.store(false, Ordering::SeqCst);
                    let _ = state.sender.send(CapturedEvent::EmergencyEscape);
                    return CallNextHookEx(None, n_code, w_param, l_param);
                }

                let swallow = state.swallow.load(Ordering::SeqCst);
                if swallow {
                    let scancode = hook_struct.scanCode as u16;
                    let extended = (hook_struct.flags.0 & 1) != 0; // LLKHF_EXTENDED = 1
                    let _ = state.sender.send(CapturedEvent::Packet(Packet::Key {
                        scancode,
                        extended,
                        pressed: is_down,
                    }));
                    return LRESULT(1);
                }
            }
        }
    }

    CallNextHookEx(None, n_code, w_param, l_param)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_capture_engine_rehook() {
        let (engine, _rx) = CaptureEngine::new();
        assert!(engine.rehook());
    }
}
