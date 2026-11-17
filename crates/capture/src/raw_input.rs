use crossbeam_channel::Sender;
use proto::{MouseBtn, Packet};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::Graphics::Gdi::HBRUSH;
use windows::Win32::UI::Input::{
    GetRawInputData, RegisterRawInputDevices, HRAWINPUT, MOUSE_MOVE_ABSOLUTE,
    RAWINPUT, RAWINPUTDEVICE, RAWINPUTHEADER, RIDEV_INPUTSINK, RIDEV_REMOVE,
    RID_INPUT, RIM_TYPEMOUSE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    PostThreadMessageW, RegisterClassExW, TranslateMessage, HCURSOR, HICON,
    HWND_MESSAGE, MSG, RI_MOUSE_BUTTON_4_DOWN, RI_MOUSE_BUTTON_4_UP,
    RI_MOUSE_BUTTON_5_DOWN, RI_MOUSE_BUTTON_5_UP, RI_MOUSE_HWHEEL,
    RI_MOUSE_LEFT_BUTTON_DOWN, RI_MOUSE_LEFT_BUTTON_UP, RI_MOUSE_MIDDLE_BUTTON_DOWN,
    RI_MOUSE_MIDDLE_BUTTON_UP, RI_MOUSE_RIGHT_BUTTON_DOWN, RI_MOUSE_RIGHT_BUTTON_UP,
    RI_MOUSE_WHEEL, WINDOW_EX_STYLE, WINDOW_STYLE, WM_INPUT, WM_QUIT, WNDCLASSEXW,
    WNDCLASS_STYLES,
};

use crate::hook::CapturedEvent;

#[derive(Debug, thiserror::Error)]
pub enum CaptureError {
    #[error("Windows API error: {0}")]
    Windows(#[from] windows::core::Error),
    #[error("Window class registration failed: error code {0}")]
    WindowClassFailed(u32),
    #[error("Failed to create message-only window: {0}")]
    WindowCreationFailed(String),
    #[error("Failed to register raw input devices: {0}")]
    DeviceRegistrationFailed(String),
    #[error("Initialization channel failed")]
    InitChannelFailed,
}

struct RawInputSinkState {
    event_tx: Sender<CapturedEvent>,
    enabled: Arc<AtomicBool>,
}

static SINK_STATE: Mutex<Option<RawInputSinkState>> = Mutex::new(None);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SendHwnd(pub HWND);
unsafe impl Send for SendHwnd {}
unsafe impl Sync for SendHwnd {}

/// Hardware-level mouse capture via Windows Raw Input (`WM_INPUT`) with `RIDEV_INPUTSINK`.
///
/// Uses a message-only window (`HWND_MESSAGE`) and dedicated message pump thread to receive
/// relative mouse deltas (`lLastX`, `lLastY`), buttons, and wheel events even while
/// the cursor is clamped/parked.
pub struct RawInputSink {
    enabled: Arc<AtomicBool>,
    thread_handle: Option<thread::JoinHandle<()>>,
    thread_id: u32,
    hwnd: SendHwnd,
}

impl RawInputSink {
    /// Creates a new `RawInputSink` that begins capturing raw input immediately when active.
    pub fn new(event_tx: Sender<CapturedEvent>) -> Result<Self, CaptureError> {
        let enabled = Arc::new(AtomicBool::new(true));
        Self::new_with_enabled(event_tx, enabled)
    }

    /// Creates a new `RawInputSink` sharing an existing `enabled` flag (e.g. coordinated with hook swallowing).
    pub fn new_with_enabled(
        event_tx: Sender<CapturedEvent>,
        enabled: Arc<AtomicBool>,
    ) -> Result<Self, CaptureError> {
        let enabled_clone = enabled.clone();
        let (init_tx, init_rx) = crossbeam_channel::bounded(1);

        let thread_handle = thread::spawn(move || {
            let hinstance = match unsafe { windows::Win32::System::LibraryLoader::GetModuleHandleW(None) } {
                Ok(h) => h,
                Err(e) => {
                    let _ = init_tx.send(Err(CaptureError::Windows(e)));
                    return;
                }
            };

            let class_name = windows::core::w!("OpenMouseRawInputSinkWindow");
            let wc = WNDCLASSEXW {
                cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                style: WNDCLASS_STYLES(0),
                lpfnWndProc: Some(raw_input_wndproc),
                cbClsExtra: 0,
                cbWndExtra: 0,
                hInstance: hinstance.into(),
                hIcon: HICON::default(),
                hCursor: HCURSOR::default(),
                hbrBackground: HBRUSH::default(),
                lpszMenuName: PCWSTR::null(),
                lpszClassName: class_name,
                hIconSm: HICON::default(),
            };

            let atom = unsafe { RegisterClassExW(&wc) };
            if atom == 0 {
                let err = unsafe { GetLastError() };
                if err != ERROR_CLASS_ALREADY_EXISTS {
                    let _ = init_tx.send(Err(CaptureError::WindowClassFailed(err.0)));
                    return;
                }
            }

            let hwnd = match unsafe {
                CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class_name,
                    windows::core::w!("OpenMouseRawInputWindow"),
                    WINDOW_STYLE(0),
                    0,
                    0,
                    0,
                    0,
                    Some(HWND_MESSAGE),
                    None,
                    Some(hinstance.into()),
                    None,
                )
            } {
                Ok(h) => h,
                Err(e) => {
                    let _ = init_tx.send(Err(CaptureError::WindowCreationFailed(e.to_string())));
                    return;
                }
            };

            let rid = RAWINPUTDEVICE {
                usUsagePage: 0x01, // Generic Desktop Controls
                usUsage: 0x02,     // Mouse
                dwFlags: RIDEV_INPUTSINK,
                hwndTarget: hwnd,
            };

            if let Err(e) = unsafe {
                RegisterRawInputDevices(&[rid], std::mem::size_of::<RAWINPUTDEVICE>() as u32)
            } {
                let _ = unsafe { DestroyWindow(hwnd) };
                let _ = init_tx.send(Err(CaptureError::DeviceRegistrationFailed(e.to_string())));
                return;
            }

            {
                let mut state = SINK_STATE.lock().unwrap();
                *state = Some(RawInputSinkState {
                    event_tx,
                    enabled: enabled_clone,
                });
            }

            let thread_id = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
            if init_tx.send(Ok((SendHwnd(hwnd), thread_id))).is_err() {
                return;
            }

            // Dedicated message pump
            let mut msg = MSG::default();
            unsafe {
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                // Cleanup on thread exit
                let rid_remove = RAWINPUTDEVICE {
                    usUsagePage: 0x01,
                    usUsage: 0x02,
                    dwFlags: RIDEV_REMOVE,
                    hwndTarget: HWND::default(),
                };
                let _ = RegisterRawInputDevices(&[rid_remove], std::mem::size_of::<RAWINPUTDEVICE>() as u32);
                let _ = DestroyWindow(hwnd);
            }
        });

        let (hwnd, thread_id) = init_rx
            .recv()
            .map_err(|_| CaptureError::InitChannelFailed)??;

        Ok(Self {
            enabled,
            thread_handle: Some(thread_handle),
            thread_id,
            hwnd,
        })
    }

    /// Sets whether raw input events are actively forwarded to the channel.
    pub fn set_enabled(&self, enabled: bool) {
        self.enabled.store(enabled, Ordering::SeqCst);
    }

    /// Queries whether raw input capture is currently active.
    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// Returns the window handle for the message-only window.
    pub fn hwnd(&self) -> HWND {
        self.hwnd.0
    }
}

impl Drop for RawInputSink {
    fn drop(&mut self) {
        self.enabled.store(false, Ordering::SeqCst);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
        if let Some(handle) = self.thread_handle.take() {
            let _ = handle.join();
        }
        if let Ok(mut state) = SINK_STATE.lock() {
            *state = None;
        }
    }
}

unsafe extern "system" fn raw_input_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_INPUT {
        handle_raw_input(lparam);
        // CRITICAL: Call DefWindowProcW for every WM_INPUT so Windows releases kernel input resources.
        return DefWindowProcW(hwnd, msg, wparam, lparam);
    }
    DefWindowProcW(hwnd, msg, wparam, lparam)
}

unsafe fn handle_raw_input(lparam: LPARAM) {
    let state_guard = match SINK_STATE.try_lock() {
        Ok(guard) => guard,
        Err(_) => return,
    };
    let state = match state_guard.as_ref() {
        Some(s) => s,
        None => return,
    };

    if !state.enabled.load(Ordering::SeqCst) {
        return;
    }

    let hrawinput = HRAWINPUT(lparam.0 as *mut core::ffi::c_void);
    let mut size: u32 = 0;
    let header_size = std::mem::size_of::<RAWINPUTHEADER>() as u32;

    let res = GetRawInputData(hrawinput, RID_INPUT, None, &mut size, header_size);
    if res == u32::MAX || size == 0 {
        return;
    }

    // 8-byte aligned buffer using Vec<u64>
    let u64_count = ((size as usize) + 7) / 8;
    let mut buffer: Vec<u64> = vec![0u64; u64_count];
    let res = GetRawInputData(
        hrawinput,
        RID_INPUT,
        Some(buffer.as_mut_ptr() as *mut core::ffi::c_void),
        &mut size,
        header_size,
    );

    if res == u32::MAX || res != size {
        return;
    }

    let raw = &*(buffer.as_ptr() as *const RAWINPUT);
    if raw.header.dwType != RIM_TYPEMOUSE.0 {
        return;
    }

    let mouse = &raw.data.mouse;

    // Relative movement check: MOUSE_MOVE_ABSOLUTE bit must be 0
    if (mouse.usFlags.0 & MOUSE_MOVE_ABSOLUTE.0) == 0 {
        let dx = mouse.lLastX;
        let dy = mouse.lLastY;
        if dx != 0 || dy != 0 {
            let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseMoveRel { dx, dy }));
        }
    }

    // Buttons and wheel
    let btn_flags = mouse.Anonymous.Anonymous.usButtonFlags;
    let btn_data = mouse.Anonymous.Anonymous.usButtonData;

    if (btn_flags & (RI_MOUSE_LEFT_BUTTON_DOWN as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::Left,
            pressed: true,
        }));
    }
    if (btn_flags & (RI_MOUSE_LEFT_BUTTON_UP as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::Left,
            pressed: false,
        }));
    }
    if (btn_flags & (RI_MOUSE_RIGHT_BUTTON_DOWN as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::Right,
            pressed: true,
        }));
    }
    if (btn_flags & (RI_MOUSE_RIGHT_BUTTON_UP as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::Right,
            pressed: false,
        }));
    }
    if (btn_flags & (RI_MOUSE_MIDDLE_BUTTON_DOWN as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::Middle,
            pressed: true,
        }));
    }
    if (btn_flags & (RI_MOUSE_MIDDLE_BUTTON_UP as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::Middle,
            pressed: false,
        }));
    }
    if (btn_flags & (RI_MOUSE_BUTTON_4_DOWN as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::X1,
            pressed: true,
        }));
    }
    if (btn_flags & (RI_MOUSE_BUTTON_4_UP as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::X1,
            pressed: false,
        }));
    }
    if (btn_flags & (RI_MOUSE_BUTTON_5_DOWN as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::X2,
            pressed: true,
        }));
    }
    if (btn_flags & (RI_MOUSE_BUTTON_5_UP as u16)) != 0 {
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseButton {
            button: MouseBtn::X2,
            pressed: false,
        }));
    }
    if (btn_flags & (RI_MOUSE_WHEEL as u16)) != 0 {
        let delta_y = btn_data as i16 as i32;
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseScroll {
            delta_x: 0,
            delta_y,
        }));
    }
    if (btn_flags & (RI_MOUSE_HWHEEL as u16)) != 0 {
        let delta_x = btn_data as i16 as i32;
        let _ = state.event_tx.send(CapturedEvent::Packet(Packet::MouseScroll {
            delta_x,
            delta_y: 0,
        }));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_raw_input_sink_lifecycle() {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let sink = RawInputSink::new(tx).expect("RawInputSink creation failed");
        assert!(sink.is_enabled());
        assert!(!sink.hwnd().0.is_null());

        sink.set_enabled(false);
        assert!(!sink.is_enabled());

        sink.set_enabled(true);
        assert!(sink.is_enabled());

        // Dropping sink should stop the thread and cleanup gracefully
        drop(sink);
    }
}
