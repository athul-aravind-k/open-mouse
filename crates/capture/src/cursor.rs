use windows::Win32::Foundation::{POINT, RECT};
use windows::Win32::UI::WindowsAndMessaging::{
    ClipCursor, GetCursorPos, GetSystemMetrics, SetCursorPos,
    SM_CXSCREEN, SM_CYSCREEN, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN,
    SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN,
};

pub fn get_cursor_pos() -> (i32, i32) {
    let mut pt = POINT { x: 0, y: 0 };
    unsafe {
        let _ = GetCursorPos(&mut pt);
    }
    (pt.x, pt.y)
}

pub fn set_cursor_pos(x: i32, y: i32) -> bool {
    unsafe { SetCursorPos(x, y).is_ok() }
}

pub fn clip_cursor_to_point(x: i32, y: i32) -> bool {
    let rect = RECT {
        left: x,
        top: y,
        right: x + 1,
        bottom: y + 1,
    };
    unsafe { ClipCursor(Some(&rect)).is_ok() }
}

pub fn unclip_cursor() -> bool {
    unsafe { ClipCursor(None).is_ok() }
}

/// Parks the cursor at the specified coordinates by positioning it and clipping it to a 1x1 point.
pub fn park_cursor(x: i32, y: i32) -> bool {
    set_cursor_pos(x, y) && clip_cursor_to_point(x, y)
}

/// Unparks the cursor by releasing cursor clipping and positioning it at the specified coordinates.
pub fn unpark_cursor(x: i32, y: i32) -> bool {
    let unclipped = unclip_cursor();
    let positioned = set_cursor_pos(x, y);
    unclipped && positioned
}

/// RAII guard that automatically releases cursor clipping when dropped.
#[derive(Debug)]
pub struct CursorClipGuard {
    active: bool,
}

impl CursorClipGuard {
    pub fn new() -> Self {
        Self { active: true }
    }

    /// Parks the cursor at (x, y) and returns a guard that will unclip it on drop.
    pub fn park(x: i32, y: i32) -> (bool, Self) {
        let success = park_cursor(x, y);
        (success, Self { active: success })
    }

    /// Explicitly dismiss the guard without unclipping.
    pub fn dismiss(&mut self) {
        self.active = false;
    }

    /// Checks if the guard is currently active.
    pub fn is_active(&self) -> bool {
        self.active
    }
}

impl Default for CursorClipGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for CursorClipGuard {
    fn drop(&mut self) {
        if self.active {
            unclip_cursor();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cursor_clip_guard_dismiss() {
        let mut guard = CursorClipGuard::new();
        assert!(guard.is_active());
        guard.dismiss();
        assert!(!guard.is_active());
    }

    #[test]
    fn test_cursor_metrics_read() {
        let (w, h) = get_primary_screen_size();
        assert!(w > 0);
        assert!(h > 0);
    }
}

pub fn get_primary_screen_size() -> (i32, i32) {
    unsafe {
        let w = GetSystemMetrics(SM_CXSCREEN);
        let h = GetSystemMetrics(SM_CYSCREEN);
        (w, h)
    }
}

pub fn get_virtual_screen_bounds() -> (i32, i32, i32, i32) {
    unsafe {
        let x = GetSystemMetrics(SM_XVIRTUALSCREEN);
        let y = GetSystemMetrics(SM_YVIRTUALSCREEN);
        let w = GetSystemMetrics(SM_CXVIRTUALSCREEN);
        let h = GetSystemMetrics(SM_CYVIRTUALSCREEN);
        (x, y, w, h)
    }
}
