use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, WPARAM};
use windows::Win32::System::RemoteDesktop::{
    WTSRegisterSessionNotification, WTSUnRegisterSessionNotification, NOTIFY_FOR_THIS_SESSION,
};
use windows::Win32::UI::Shell::{
    Shell_NotifyIconW, NIF_ICON, NIF_INFO, NIF_MESSAGE, NIF_TIP, NIM_ADD, NIM_DELETE, NIM_MODIFY,
    NOTIFYICONDATAW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreatePopupMenu, CreateWindowExW, DefWindowProcW, DestroyMenu,
    DispatchMessageW, GetCursorPos, GetMessageW, LoadIconW, PostQuitMessage,
    RegisterClassW, SetForegroundWindow, TrackPopupMenu, TranslateMessage,
    HICON, IDI_APPLICATION, MF_DISABLED, MF_GRAYED, MF_SEPARATOR, MF_STRING,
    MSG, TPM_BOTTOMALIGN, TPM_LEFTALIGN, TPM_RIGHTBUTTON, WINDOW_EX_STYLE,
    WM_COMMAND, WM_DESTROY, WM_LBUTTONUP, WM_RBUTTONUP, WM_USER, WNDCLASSW, WS_OVERLAPPED,
    WM_POWERBROADCAST, PBT_APMSUSPEND, PBT_APMRESUMEAUTOMATIC, PBT_APMRESUMESUSPEND,
    WM_WTSSESSION_CHANGE, WTS_SESSION_LOCK, WTS_SESSION_UNLOCK,
    WM_DISPLAYCHANGE,
};

pub const WM_TRAY_ICON: u32 = WM_USER + 101;

pub const ID_STATUS_HEADER: usize = 2001;
pub const ID_CONFIG_EDIT: usize = 2002;
pub const ID_SWITCH_EDGE: usize = 2003;
pub const ID_UNLOCK: usize = 2004;
pub const ID_AUTOSTART: usize = 2005;
pub const ID_EXIT: usize = 2006;
pub const ID_ARRANGE_DISPLAYS: usize = 2007;
pub const ID_SETUP_FIREWALL: usize = 2008;
pub const ID_VIEW_LOGS: usize = 2009;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayEvent {
    Command(usize),
    PowerSuspend,
    PowerResume,
    SessionLock,
    SessionUnlock,
    DisplayChange { width: u32, height: u32 },
}

pub struct TrayManager {
    nid: NOTIFYICONDATAW,
}

type TrayEventCallback = Box<dyn Fn(TrayEvent) + Send + Sync>;
static TRAY_EVENT_CALLBACK: Mutex<Option<TrayEventCallback>> = Mutex::new(None);
pub static CURRENT_STATUS_TEXT: Mutex<Option<String>> = Mutex::new(None);
pub static CURRENT_EDGE_TEXT: Mutex<Option<String>> = Mutex::new(None);
pub static AUTOSTART_ENABLED: Mutex<bool> = Mutex::new(false);

impl TrayManager {
    pub fn new<F>(title: &str, on_event: F) -> Self
    where
        F: Fn(TrayEvent) + Send + Sync + 'static,
    {
        *TRAY_EVENT_CALLBACK.lock().unwrap() = Some(Box::new(on_event));

        let class_name = w!("OpenMouseTrayClass");
        let icon = unsafe { LoadIconW(None, IDI_APPLICATION).unwrap_or(HICON(std::ptr::null_mut())) };

        let wc = WNDCLASSW {
            lpfnWndProc: Some(wnd_proc),
            lpszClassName: class_name,
            hIcon: icon,
            ..Default::default()
        };
        unsafe {
            let _ = RegisterClassW(&wc);
        }

        let hwnd = unsafe {
            CreateWindowExW(
                WINDOW_EX_STYLE(0),
                class_name,
                w!("OpenMouseTrayWindow"),
                WS_OVERLAPPED,
                0,
                0,
                0,
                0,
                None,
                None,
                None,
                None,
            ).expect("Failed to create tray hidden window")
        };

        unsafe {
            let _ = WTSRegisterSessionNotification(hwnd, NOTIFY_FOR_THIS_SESSION);
        }

        let mut nid = NOTIFYICONDATAW {
            cbSize: std::mem::size_of::<NOTIFYICONDATAW>() as u32,
            hWnd: hwnd,
            uID: 1,
            uFlags: NIF_ICON | NIF_MESSAGE | NIF_TIP,
            uCallbackMessage: WM_TRAY_ICON,
            hIcon: icon,
            ..Default::default()
        };

        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let copy_len = title_wide.len().min(nid.szTip.len());
        nid.szTip[..copy_len].copy_from_slice(&title_wide[..copy_len]);

        unsafe {
            let _ = Shell_NotifyIconW(NIM_ADD, &nid);
        }

        Self { nid }
    }

    pub fn show_balloon(&mut self, title: &str, info: &str) {
        self.nid.uFlags = NIF_INFO;
        let title_wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
        let info_wide: Vec<u16> = info.encode_utf16().chain(std::iter::once(0)).collect();

        let t_len = title_wide.len().min(self.nid.szInfoTitle.len());
        self.nid.szInfoTitle = [0; 64];
        self.nid.szInfoTitle[..t_len].copy_from_slice(&title_wide[..t_len]);

        let i_len = info_wide.len().min(self.nid.szInfo.len());
        self.nid.szInfo = [0; 256];
        self.nid.szInfo[..i_len].copy_from_slice(&info_wide[..i_len]);

        unsafe {
            let _ = Shell_NotifyIconW(NIM_MODIFY, &self.nid);
        }
    }
}

impl Drop for TrayManager {
    fn drop(&mut self) {
        unsafe {
            let _ = WTSUnRegisterSessionNotification(self.nid.hWnd);
            let _ = Shell_NotifyIconW(NIM_DELETE, &self.nid);
        }
    }
}

unsafe extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_TRAY_ICON => {
            let event = lparam.0 as u32;
            if event == WM_RBUTTONUP || event == WM_LBUTTONUP {
                show_context_menu(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0;
            if let Some(ref cb) = *TRAY_EVENT_CALLBACK.lock().unwrap() {
                cb(TrayEvent::Command(id));
            }
            LRESULT(0)
        }
        WM_POWERBROADCAST => {
            let pbt = wparam.0 as u32;
            if pbt == PBT_APMSUSPEND {
                capture::unclip_cursor();
                if let Some(ref cb) = *TRAY_EVENT_CALLBACK.lock().unwrap() {
                    cb(TrayEvent::PowerSuspend);
                }
            } else if pbt == PBT_APMRESUMEAUTOMATIC || pbt == PBT_APMRESUMESUSPEND {
                if let Some(ref cb) = *TRAY_EVENT_CALLBACK.lock().unwrap() {
                    cb(TrayEvent::PowerResume);
                }
            }
            LRESULT(1)
        }
        WM_WTSSESSION_CHANGE => {
            let wts = wparam.0 as u32;
            if wts == WTS_SESSION_LOCK {
                capture::unclip_cursor();
                if let Some(ref cb) = *TRAY_EVENT_CALLBACK.lock().unwrap() {
                    cb(TrayEvent::SessionLock);
                }
            } else if wts == WTS_SESSION_UNLOCK {
                if let Some(ref cb) = *TRAY_EVENT_CALLBACK.lock().unwrap() {
                    cb(TrayEvent::SessionUnlock);
                }
            }
            LRESULT(0)
        }
        WM_DISPLAYCHANGE => {
            let (w, h) = capture::get_primary_screen_size();
            let width = if w > 0 { w as u32 } else { (lparam.0 & 0xFFFF) as u32 };
            let height = if h > 0 { h as u32 } else { ((lparam.0 >> 16) & 0xFFFF) as u32 };
            if let Some(ref cb) = *TRAY_EVENT_CALLBACK.lock().unwrap() {
                cb(TrayEvent::DisplayChange { width, height });
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = WTSUnRegisterSessionNotification(hwnd);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn show_context_menu(hwnd: HWND) {
    let mut pt = POINT { x: 0, y: 0 };
    let _ = GetCursorPos(&mut pt);

    let menu = match CreatePopupMenu() {
        Ok(m) => m,
        Err(_) => return,
    };

    let status_str = {
        let lock = CURRENT_STATUS_TEXT.lock().unwrap();
        lock.clone().unwrap_or_else(|| "Status: Running".to_string())
    };
    let status_wide: Vec<u16> = status_str.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, ID_STATUS_HEADER, PCWSTR(status_wide.as_ptr()));

    let local_ip_str = format!("Local IP: {}\0", daemon::get_local_ip());
    let local_ip_wide: Vec<u16> = local_ip_str.encode_utf16().collect();
    let _ = AppendMenuW(menu, MF_STRING | MF_DISABLED | MF_GRAYED, 2010, PCWSTR(local_ip_wide.as_ptr()));

    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

    let arrange_str = "Arrange Displays...\0".encode_utf16().collect::<Vec<_>>();
    let _ = AppendMenuW(menu, MF_STRING, ID_ARRANGE_DISPLAYS, PCWSTR(arrange_str.as_ptr()));

    let cfg_str = "Edit Config (Peer IP & Settings)\0".encode_utf16().collect::<Vec<_>>();
    let _ = AppendMenuW(menu, MF_STRING, ID_CONFIG_EDIT, PCWSTR(cfg_str.as_ptr()));

    let edge_str = {
        let lock = CURRENT_EDGE_TEXT.lock().unwrap();
        lock.clone().unwrap_or_else(|| "Switch Shared Edge".to_string())
    };
    let edge_wide: Vec<u16> = edge_str.encode_utf16().chain(std::iter::once(0)).collect();
    let _ = AppendMenuW(menu, MF_STRING, ID_SWITCH_EDGE, PCWSTR(edge_wide.as_ptr()));

    let unlock_str = "Emergency Unlock (Ctrl+Alt+Esc)\0".encode_utf16().collect::<Vec<_>>();
    let _ = AppendMenuW(menu, MF_STRING, ID_UNLOCK, PCWSTR(unlock_str.as_ptr()));

    let autostart_enabled = *AUTOSTART_ENABLED.lock().unwrap();
    let autostart_str = if autostart_enabled {
        "Start with Windows: Enabled\0".encode_utf16().collect::<Vec<_>>()
    } else {
        "Start with Windows: Disabled\0".encode_utf16().collect::<Vec<_>>()
    };
    let _ = AppendMenuW(menu, MF_STRING, ID_AUTOSTART, PCWSTR(autostart_str.as_ptr()));

    let fw_str = "Configure Firewall...\0".encode_utf16().collect::<Vec<_>>();
    let _ = AppendMenuW(menu, MF_STRING, ID_SETUP_FIREWALL, PCWSTR(fw_str.as_ptr()));

    let log_str = "View Logs...\0".encode_utf16().collect::<Vec<_>>();
    let _ = AppendMenuW(menu, MF_STRING, ID_VIEW_LOGS, PCWSTR(log_str.as_ptr()));

    let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());

    let exit_str = "Exit OpenMouse\0".encode_utf16().collect::<Vec<_>>();
    let _ = AppendMenuW(menu, MF_STRING, ID_EXIT, PCWSTR(exit_str.as_ptr()));

    let _ = SetForegroundWindow(hwnd);
    let _ = TrackPopupMenu(
        menu,
        TPM_LEFTALIGN | TPM_BOTTOMALIGN | TPM_RIGHTBUTTON,
        pt.x,
        pt.y,
        Some(0),
        hwnd,
        None,
    );
    let _ = DestroyMenu(menu);
}

pub fn run_message_loop(running: Arc<AtomicBool>) {
    let mut msg = MSG::default();
    unsafe {
        while running.load(Ordering::SeqCst) {
            let ret = GetMessageW(&mut msg, None, 0, 0);
            if ret.0 <= 0 {
                crate::append_log(&format!("GetMessageW returned {} (msg=0x{:X})", ret.0, msg.message));
                break;
            }
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }
    }
}
