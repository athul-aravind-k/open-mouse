use std::thread;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetStockObject, SelectObject, SetBkMode, SetTextColor, DT_CENTER,
    DT_SINGLELINE, DT_VCENTER, FONT_CHARSET, FONT_CLIP_PRECISION, FONT_OUTPUT_PRECISION,
    FONT_QUALITY, FW_BOLD, HBRUSH, HGDIOBJ, PAINTSTRUCT, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW,
    GetSystemMetrics, KillTimer, PostQuitMessage, RegisterClassW, SetLayeredWindowAttributes,
    SetTimer, ShowWindow, TranslateMessage, LWA_ALPHA, MSG, SM_CXSCREEN, SM_CYSCREEN,
    WM_DESTROY, WM_PAINT, WM_TIMER, WNDCLASSW, WS_EX_LAYERED, WS_EX_TOOLWINDOW,
    WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP, SW_SHOW,
};

pub fn show_identify_overlay(display_number: u32, duration_ms: u32) {
    thread::spawn(move || unsafe {
        let class_name = w!("OpenMouseIdentifyOverlayClass");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(overlay_wnd_proc),
            lpszClassName: class_name,
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let win_size = 280;
        let x = (screen_w - win_size) / 2;
        let y = (screen_h - win_size) / 2;

        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            class_name,
            w!("IdentifyOverlay"),
            WS_POPUP,
            x,
            y,
            win_size,
            win_size,
            None,
            None,
            None,
            Some(display_number as usize as *const std::ffi::c_void),
        );

        if let Ok(hwnd) = hwnd {
            let _ = SetLayeredWindowAttributes(hwnd, COLORREF(0), 220, LWA_ALPHA);
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetTimer(Some(hwnd), 1, duration_ms, None);

            let mut msg = MSG::default();
            while GetMessageW(&mut msg, None, 0, 0).as_bool() {
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }
    });
}

static DISPLAY_NUMBERS: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(1);

unsafe extern "system" fn overlay_wnd_proc(
    hwnd: HWND,
    msg: u32,
    _wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        windows::Win32::UI::WindowsAndMessaging::WM_CREATE => {
            let create_struct = *(lparam.0 as *const windows::Win32::UI::WindowsAndMessaging::CREATESTRUCTW);
            let num = create_struct.lpCreateParams as usize as u32;
            DISPLAY_NUMBERS.store(num, std::sync::atomic::Ordering::SeqCst);
            LRESULT(0)
        }
        WM_TIMER => {
            let _ = KillTimer(Some(hwnd), 1);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut rect = RECT {
                left: 0,
                top: 0,
                right: 280,
                bottom: 280,
            };

            // Dark slate background: RGB(30, 34, 42) -> 0x002A221E
            let bg_brush = CreateSolidBrush(COLORREF(0x002A221E));
            FillRect(hdc, &rect, bg_brush);
            let _ = DeleteObject(HGDIOBJ(bg_brush.0));

            // Large crisp font
            let font = CreateFontW(
                160,
                0,
                0,
                0,
                FW_BOLD.0 as i32,
                0,
                0,
                0,
                FONT_CHARSET(0),
                FONT_OUTPUT_PRECISION(0),
                FONT_CLIP_PRECISION(0),
                FONT_QUALITY(0),
                0,
                w!("Segoe UI"),
            );
            let old_font = SelectObject(hdc, font.into());

            SetBkMode(hdc, TRANSPARENT);
            SetTextColor(hdc, COLORREF(0x00FFFFFF));

            let num = DISPLAY_NUMBERS.load(std::sync::atomic::Ordering::SeqCst);
            let text = format!("{}", num);
            let mut wide: Vec<u16> = text.encode_utf16().collect();

            DrawTextW(hdc, &mut wide[..], &mut rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

            SelectObject(hdc, old_font);
            let _ = DeleteObject(HGDIOBJ(font.0));

            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, _wparam, lparam),
    }
}
