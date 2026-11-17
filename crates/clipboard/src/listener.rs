use crossbeam_channel::{Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use windows::core::w;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::DataExchange::{AddClipboardFormatListener, RemoveClipboardFormatListener};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetMessageW, PostQuitMessage,
    PostThreadMessageW, RegisterClassW, TranslateMessage, MSG, WINDOW_EX_STYLE, WM_CLIPBOARDUPDATE,
    WM_DESTROY, WM_QUIT, WNDCLASSW, WS_OVERLAPPED,
};

pub struct ClipboardListener {
    running: Arc<AtomicBool>,
    thread_id: u32,
    rx: Receiver<()>,
}

static mut LISTENER_TX: Option<Sender<()>> = None;

impl ClipboardListener {
    pub fn new() -> Self {
        let (tx, rx) = crossbeam_channel::unbounded();
        let (ready_tx, ready_rx) = crossbeam_channel::bounded(1);
        let running = Arc::new(AtomicBool::new(true));

        thread::spawn(move || {
            unsafe {
                LISTENER_TX = Some(tx);

                let class_name = w!("OpenMouseClipboardListenerClass");
                let wc = WNDCLASSW {
                    lpfnWndProc: Some(listener_wnd_proc),
                    lpszClassName: class_name,
                    ..Default::default()
                };
                let _ = RegisterClassW(&wc);

                let hwnd = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    class_name,
                    w!("OpenMouseClipboardWindow"),
                    WS_OVERLAPPED,
                    0,
                    0,
                    0,
                    0,
                    None,
                    None,
                    None,
                    None,
                ).expect("Failed to create clipboard listener window");

                let _ = AddClipboardFormatListener(hwnd);

                let thread_id = windows::Win32::System::Threading::GetCurrentThreadId();
                let _ = ready_tx.send(thread_id);

                let mut msg = MSG::default();
                while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                let _ = RemoveClipboardFormatListener(hwnd);
            }
        });

        let thread_id = ready_rx.recv().expect("Failed to initialize clipboard listener thread");

        Self {
            running,
            thread_id,
            rx,
        }
    }

    pub fn receiver(&self) -> Receiver<()> {
        self.rx.clone()
    }
}

impl Drop for ClipboardListener {
    fn drop(&mut self) {
        self.running.store(false, Ordering::SeqCst);
        unsafe {
            let _ = PostThreadMessageW(self.thread_id, WM_QUIT, WPARAM(0), LPARAM(0));
        }
    }
}

unsafe extern "system" fn listener_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CLIPBOARDUPDATE => {
            if let Some(ref tx) = LISTENER_TX {
                let _ = tx.send(());
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
