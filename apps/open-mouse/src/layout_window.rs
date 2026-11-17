use daemon::{DisplayArrangement, EdgeDirection, PeerNode};
use std::sync::{Arc, Mutex};
use std::thread;
use windows::core::w;
use windows::Win32::Foundation::{COLORREF, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreateSolidBrush, DeleteDC, DeleteObject, DrawTextW, EndPaint, FillRect, FrameRect,
    GetStockObject, InvalidateRect, SelectObject, SetBkMode, SetTextColor,
    DT_CENTER, DT_SINGLELINE, DT_VCENTER, FONT_CHARSET, FONT_CLIP_PRECISION,
    FONT_OUTPUT_PRECISION, FONT_QUALITY, FW_BOLD, FW_NORMAL, HBRUSH, HGDIOBJ,
    PAINTSTRUCT, SRCCOPY, TRANSPARENT, WHITE_BRUSH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetMessageW,
    GetSystemMetrics, KillTimer, PostQuitMessage, RegisterClassW, SetForegroundWindow,
    SetTimer, SetWindowTextW, ShowWindow, TranslateMessage, BS_DEFPUSHBUTTON,
    BS_PUSHBUTTON, HMENU, MSG, SM_CXSCREEN, SM_CYSCREEN, SW_SHOW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WM_COMMAND, WM_DESTROY, WM_ERASEBKGND, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_PAINT, WM_TIMER, WNDCLASSW, WS_CAPTION, WS_CHILD, WS_MINIMIZEBOX,
    WS_OVERLAPPED, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const ID_BTN_IDENTIFY: usize = 3001;
const ID_BTN_APPLY: usize = 3002;

static LAYOUT_WINDOW_HWND: Mutex<Option<isize>> = Mutex::new(None);
static CURRENT_ARRANGEMENT: Mutex<DisplayArrangement> = Mutex::new(DisplayArrangement {
    relative_edge: EdgeDirection::Right,
    offset_x: 0,
    offset_y: 0,
});
static PEER_NODE_REF: Mutex<Option<Arc<PeerNode>>> = Mutex::new(None);
static SCREEN_DIMS: Mutex<(u32, u32, u32, u32)> = Mutex::new((1920, 1080, 1920, 1080)); // w1, h1, w2, h2

// Drag state
static IS_DRAGGING: Mutex<bool> = Mutex::new(false);
static DRAG_START_MOUSE: Mutex<POINT> = Mutex::new(POINT { x: 0, y: 0 });
static DRAG_START_ARRANGEMENT: Mutex<DisplayArrangement> = Mutex::new(DisplayArrangement {
    relative_edge: EdgeDirection::Right,
    offset_x: 0,
    offset_y: 0,
});

pub fn open_layout_window(peer_node: Arc<PeerNode>) {
    let lock = LAYOUT_WINDOW_HWND.lock().unwrap();
    if let Some(hwnd_raw) = *lock {
        unsafe {
            let _ = SetForegroundWindow(HWND(hwnd_raw as *mut _));
        }
        return;
    }

    *PEER_NODE_REF.lock().unwrap() = Some(peer_node.clone());

    let cfg = peer_node.get_config();
    *CURRENT_ARRANGEMENT.lock().unwrap() = cfg.display_arrangement.clone();

    // Get screen dimensions
    let (w1, h1) = capture::get_primary_screen_size();
    let (w2, h2) = if let Some(remote) = peer_node.get_remote_screen() {
        (remote.width_px, remote.height_px)
    } else {
        (w1 as u32, h1 as u32)
    };
    *SCREEN_DIMS.lock().unwrap() = (w1 as u32, h1 as u32, w2, h2);

    thread::spawn(move || unsafe {
        let class_name = w!("OpenMouseLayoutWindowClass");
        let wc = WNDCLASSW {
            lpfnWndProc: Some(layout_wnd_proc),
            lpszClassName: class_name,
            hbrBackground: HBRUSH(GetStockObject(WHITE_BRUSH).0),
            ..Default::default()
        };
        let _ = RegisterClassW(&wc);

        let screen_w = GetSystemMetrics(SM_CXSCREEN);
        let screen_h = GetSystemMetrics(SM_CYSCREEN);

        let win_w = 680;
        let win_h = 490;
        let pos_x = (screen_w - win_w) / 2;
        let pos_y = (screen_h - win_h) / 2;

        let hwnd = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            class_name,
            w!("OpenMouse - Display Arrangement"),
            WS_OVERLAPPED | WS_CAPTION | WS_SYSMENU | WS_MINIMIZEBOX,
            pos_x,
            pos_y,
            win_w,
            win_h,
            None,
            None,
            None,
            None,
        ) {
            Ok(h) => h,
            Err(_) => return,
        };

        *LAYOUT_WINDOW_HWND.lock().unwrap() = Some(hwnd.0 as isize);

        // Child controls: Buttons
        let btn_style = WINDOW_STYLE((WS_TABSTOP | WS_VISIBLE | WS_CHILD).0 | (BS_PUSHBUTTON as u32));
        let btn_def_style = WINDOW_STYLE((WS_TABSTOP | WS_VISIBLE | WS_CHILD).0 | (BS_DEFPUSHBUTTON as u32));

        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("Identify Displays"),
            btn_style,
            24,
            405,
            160,
            34,
            Some(hwnd),
            Some(HMENU(ID_BTN_IDENTIFY as *mut _)),
            None,
            None,
        );

        let _ = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("BUTTON"),
            w!("Apply / Save"),
            btn_def_style,
            480,
            405,
            160,
            34,
            Some(hwnd),
            Some(HMENU(ID_BTN_APPLY as *mut _)),
            None,
            None,
        );

        let _ = ShowWindow(hwnd, SW_SHOW);
        let _ = SetForegroundWindow(hwnd);
        let _ = SetTimer(Some(hwnd), 99, 500, None);

        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        *LAYOUT_WINDOW_HWND.lock().unwrap() = None;
    });
}

const CANVAS_X: i32 = 24;
const CANVAS_Y: i32 = 24;
const CANVAS_W: i32 = 616;
const CANVAS_H: i32 = 360;
const SCALE: f32 = 0.10; // 1920x1080 -> 192x108

unsafe extern "system" fn layout_wnd_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_ERASEBKGND => LRESULT(1), // Prevent flicker
        WM_COMMAND => {
            let id = wparam.0 as usize;
            match id {
                ID_BTN_IDENTIFY => {
                    if let Some(ref peer) = *PEER_NODE_REF.lock().unwrap() {
                        peer.trigger_identify();
                    }
                }
                ID_BTN_APPLY => {
                    let arr = CURRENT_ARRANGEMENT.lock().unwrap().clone();
                    if let Some(ref peer) = *PEER_NODE_REF.lock().unwrap() {
                        peer.update_arrangement(arr);
                    }
                    let _ = SetWindowTextW(hwnd, w!("OpenMouse - Display Arrangement (Saved!)"));
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let x = (lparam.0 & 0xFFFF) as i16 as i32;
            let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;

            let (m2_rect, _) = get_monitor_canvas_rects();
            if x >= m2_rect.left && x <= m2_rect.right && y >= m2_rect.top && y <= m2_rect.bottom {
                *IS_DRAGGING.lock().unwrap() = true;
                *DRAG_START_MOUSE.lock().unwrap() = POINT { x, y };
                *DRAG_START_ARRANGEMENT.lock().unwrap() = CURRENT_ARRANGEMENT.lock().unwrap().clone();
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if *IS_DRAGGING.lock().unwrap() {
                let x = (lparam.0 & 0xFFFF) as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as i16 as i32;
                let start_pt = *DRAG_START_MOUSE.lock().unwrap();
                let start_arr = DRAG_START_ARRANGEMENT.lock().unwrap().clone();

                let delta_canvas_x = x - start_pt.x;
                let delta_canvas_y = y - start_pt.y;

                let delta_real_x = (delta_canvas_x as f32 / SCALE) as i32;
                let delta_real_y = (delta_canvas_y as f32 / SCALE) as i32;

                let (w1, h1, w2, h2) = *SCREEN_DIMS.lock().unwrap();

                // Compute proposed center displacement relative to Monitor 1
                let base_center_x = match start_arr.relative_edge {
                    EdgeDirection::Right => (w1 as i32 + w2 as i32) / 2,
                    EdgeDirection::Left => -((w1 as i32 + w2 as i32) / 2),
                    EdgeDirection::Top | EdgeDirection::Bottom => start_arr.offset_x,
                };
                let base_center_y = match start_arr.relative_edge {
                    EdgeDirection::Bottom => (h1 as i32 + h2 as i32) / 2,
                    EdgeDirection::Top => -((h1 as i32 + h2 as i32) / 2),
                    EdgeDirection::Left | EdgeDirection::Right => start_arr.offset_y,
                };

                let new_center_x = base_center_x + delta_real_x;
                let new_center_y = base_center_y + delta_real_y;

                // Determine quadrant / dominant edge
                let new_edge = if new_center_x.abs() >= new_center_y.abs() {
                    if new_center_x >= 0 {
                        EdgeDirection::Right
                    } else {
                        EdgeDirection::Left
                    }
                } else if new_center_y >= 0 {
                    EdgeDirection::Bottom
                } else {
                    EdgeDirection::Top
                };

                // Compute offsets with magnetic snapping
                let (new_offset_x, new_offset_y) = match new_edge {
                    EdgeDirection::Right | EdgeDirection::Left => {
                        let mut off_y = new_center_y;
                        if off_y.abs() < 120 {
                            off_y = 0;
                        }
                        let center_align = (h1 as i32 - h2 as i32) / 2;
                        if (off_y - center_align).abs() < 120 {
                            off_y = center_align;
                        }
                        (0, off_y)
                    }
                    EdgeDirection::Top | EdgeDirection::Bottom => {
                        let mut off_x = new_center_x;
                        if off_x.abs() < 120 {
                            off_x = 0;
                        }
                        let center_align = (w1 as i32 - w2 as i32) / 2;
                        if (off_x - center_align).abs() < 120 {
                            off_x = center_align;
                        }
                        (off_x, 0)
                    }
                };

                *CURRENT_ARRANGEMENT.lock().unwrap() = DisplayArrangement {
                    relative_edge: new_edge,
                    offset_x: new_offset_x,
                    offset_y: new_offset_y,
                };

                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            *IS_DRAGGING.lock().unwrap() = false;
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);

            let mut client_rect = RECT::default();
            let _ = GetClientRect(hwnd, &mut client_rect);
            let width = client_rect.right - client_rect.left;
            let height = client_rect.bottom - client_rect.top;

            let mem_dc = CreateCompatibleDC(Some(hdc));
            let mem_bmp = CreateCompatibleBitmap(hdc, width, height);
            let old_bmp = SelectObject(mem_dc, mem_bmp.into());

            // 1. Window background
            let bg_brush = CreateSolidBrush(COLORREF(0x00F3F3F3));
            FillRect(mem_dc, &client_rect, bg_brush);
            let _ = DeleteObject(HGDIOBJ(bg_brush.0));

            // 2. Canvas surface
            let canvas_rect = RECT {
                left: CANVAS_X,
                top: CANVAS_Y,
                right: CANVAS_X + CANVAS_W,
                bottom: CANVAS_Y + CANVAS_H,
            };
            let canvas_brush = CreateSolidBrush(COLORREF(0x00201B18)); // Dark slate
            FillRect(mem_dc, &canvas_rect, canvas_brush);
            let _ = DeleteObject(HGDIOBJ(canvas_brush.0));

            // 3. Draw Monitors
            let (m2_rect, m1_rect) = get_monitor_canvas_rects();

            // Font for monitor labels
            let title_font = CreateFontW(
                18,
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
            let sub_font = CreateFontW(
                13,
                0,
                0,
                0,
                FW_NORMAL.0 as i32,
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
            let old_font = SelectObject(mem_dc, title_font.into());
            SetBkMode(mem_dc, TRANSPARENT);

            // Draw Monitor 1 (Local)
            let m1_brush = CreateSolidBrush(COLORREF(0x003A2E1A)); // Dark teal/blue
            let m1_pen_brush = CreateSolidBrush(COLORREF(0x00D08020)); // Bright cyan-blue border
            FillRect(mem_dc, &m1_rect, m1_brush);
            FrameRect(mem_dc, &m1_rect, m1_pen_brush);
            let _ = DeleteObject(HGDIOBJ(m1_brush.0));
            let _ = DeleteObject(HGDIOBJ(m1_pen_brush.0));

            SetTextColor(mem_dc, COLORREF(0x00FFFFFF));
            let (w1, h1, w2, h2) = *SCREEN_DIMS.lock().unwrap();
            let mut label1_rect = m1_rect;
            label1_rect.bottom = (m1_rect.top + m1_rect.bottom) / 2 + 6;
            let mut text1 = "1: Local".encode_utf16().collect::<Vec<u16>>();
            DrawTextW(mem_dc, &mut text1[..], &mut label1_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

            SelectObject(mem_dc, sub_font.into());
            SetTextColor(mem_dc, COLORREF(0x00CCCCCC));
            let mut res1_wide = format!("{} x {}", w1, h1).encode_utf16().collect::<Vec<u16>>();
            let mut sub1_rect = m1_rect;
            sub1_rect.top = label1_rect.bottom - 4;
            DrawTextW(mem_dc, &mut res1_wide[..], &mut sub1_rect, DT_CENTER | DT_SINGLELINE);

            // Draw Monitor 2 (Remote)
            SelectObject(mem_dc, title_font.into());
            let is_drag = *IS_DRAGGING.lock().unwrap();
            let m2_border_color = if is_drag {
                COLORREF(0x0050D0FF) // Highlighted gold while dragging
            } else {
                COLORREF(0x0020B0F0) // Golden amber border
            };
            let m2_brush = CreateSolidBrush(COLORREF(0x001E2536)); // Warm slate
            let m2_pen_brush = CreateSolidBrush(m2_border_color);
            FillRect(mem_dc, &m2_rect, m2_brush);
            FrameRect(mem_dc, &m2_rect, m2_pen_brush);
            let _ = DeleteObject(HGDIOBJ(m2_brush.0));
            let _ = DeleteObject(HGDIOBJ(m2_pen_brush.0));

            SetTextColor(mem_dc, COLORREF(0x00FFFFFF));
            let mut label2_rect = m2_rect;
            label2_rect.bottom = (m2_rect.top + m2_rect.bottom) / 2 + 6;
            let mut text2 = "2: Remote (Drag)".encode_utf16().collect::<Vec<u16>>();
            DrawTextW(mem_dc, &mut text2[..], &mut label2_rect, DT_CENTER | DT_VCENTER | DT_SINGLELINE);

            SelectObject(mem_dc, sub_font.into());
            SetTextColor(mem_dc, COLORREF(0x00CCCCCC));
            let mut res2_wide = format!("{} x {}", w2, h2).encode_utf16().collect::<Vec<u16>>();
            let mut sub2_rect = m2_rect;
            sub2_rect.top = label2_rect.bottom - 4;
            DrawTextW(mem_dc, &mut res2_wide[..], &mut sub2_rect, DT_CENTER | DT_SINGLELINE);

            // Instruction text below canvas
            SetTextColor(mem_dc, COLORREF(0x00555555));
            let mut hint_rect = RECT {
                left: 195,
                top: 412,
                right: 470,
                bottom: 440,
            };
            let mut hint_wide = "Drag Monitor 2 to match desk setup.\nSnaps magnetically to edges.".encode_utf16().collect::<Vec<u16>>();
            DrawTextW(mem_dc, &mut hint_wide[..], &mut hint_rect, DT_CENTER);

            SelectObject(mem_dc, old_font);
            let _ = DeleteObject(HGDIOBJ(title_font.0));
            let _ = DeleteObject(HGDIOBJ(sub_font.0));

            // Blit mem_dc to screen
            let _ = BitBlt(hdc, 0, 0, width, height, Some(mem_dc), 0, 0, SRCCOPY);

            SelectObject(mem_dc, old_bmp);
            let _ = DeleteObject(HGDIOBJ(mem_bmp.0));
            let _ = DeleteDC(mem_dc);
            let _ = EndPaint(hwnd, &ps);
            LRESULT(0)
        }
        WM_TIMER => {
            if let Some(ref peer) = *PEER_NODE_REF.lock().unwrap() {
                if let Some(remote) = peer.get_remote_screen() {
                    let mut dims = SCREEN_DIMS.lock().unwrap();
                    if dims.2 != remote.width_px || dims.3 != remote.height_px {
                        dims.2 = remote.width_px;
                        dims.3 = remote.height_px;
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            let _ = KillTimer(Some(hwnd), 99);
            PostQuitMessage(0);
            LRESULT(0)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn get_monitor_canvas_rects() -> (RECT, RECT) {
    if let Some(ref peer) = *PEER_NODE_REF.lock().unwrap() {
        if let Some(remote) = peer.get_remote_screen() {
            let mut dims = SCREEN_DIMS.lock().unwrap();
            dims.2 = remote.width_px;
            dims.3 = remote.height_px;
        }
    }
    let (w1, h1, w2, h2) = *SCREEN_DIMS.lock().unwrap();
    let arr = CURRENT_ARRANGEMENT.lock().unwrap().clone();

    let m1_w = (w1 as f32 * SCALE) as i32;
    let m1_h = (h1 as f32 * SCALE) as i32;
    let m2_w = (w2 as f32 * SCALE) as i32;
    let m2_h = (h2 as f32 * SCALE) as i32;

    let center_x = CANVAS_X + CANVAS_W / 2;
    let center_y = CANVAS_Y + CANVAS_H / 2;

    let (m1_x, m1_y) = match arr.relative_edge {
        EdgeDirection::Right => (center_x - m1_w, center_y - m1_h / 2),
        EdgeDirection::Left => (center_x, center_y - m1_h / 2),
        EdgeDirection::Bottom => (center_x - m1_w / 2, center_y - m1_h),
        EdgeDirection::Top => (center_x - m1_w / 2, center_y),
    };

    let m1_rect = RECT {
        left: m1_x,
        top: m1_y,
        right: m1_x + m1_w,
        bottom: m1_y + m1_h,
    };

    let (m2_x, m2_y) = match arr.relative_edge {
        EdgeDirection::Right => (
            m1_rect.right,
            m1_rect.top + (arr.offset_y as f32 * SCALE) as i32,
        ),
        EdgeDirection::Left => (
            m1_rect.left - m2_w,
            m1_rect.top + (arr.offset_y as f32 * SCALE) as i32,
        ),
        EdgeDirection::Top => (
            m1_rect.left + (arr.offset_x as f32 * SCALE) as i32,
            m1_rect.top - m2_h,
        ),
        EdgeDirection::Bottom => (
            m1_rect.left + (arr.offset_x as f32 * SCALE) as i32,
            m1_rect.bottom,
        ),
    };

    let m2_rect = RECT {
        left: m2_x,
        top: m2_y,
        right: m2_x + m2_w,
        bottom: m2_y + m2_h,
    };

    (m2_rect, m1_rect)
}
