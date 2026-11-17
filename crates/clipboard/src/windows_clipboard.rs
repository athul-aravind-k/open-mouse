use std::ptr;
use std::thread;
use std::time::Duration;
use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};

pub const CF_DIB: u32 = 8;
pub const CF_UNICODETEXT: u32 = 13;

pub struct ClipboardGuard;

impl ClipboardGuard {
    pub fn open() -> Option<Self> {
        // Retry loop: clipboard may be temporarily held by another process
        for _ in 0..5 {
            unsafe {
                if OpenClipboard(None).is_ok() {
                    return Some(ClipboardGuard);
                }
            }
            thread::sleep(Duration::from_millis(10));
        }
        None
    }
}

impl Drop for ClipboardGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = CloseClipboard();
        }
    }
}

pub fn read_clipboard_text() -> Option<String> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        if !IsClipboardFormatAvailable(CF_UNICODETEXT).is_ok() {
            return None;
        }

        let handle = match GetClipboardData(CF_UNICODETEXT) {
            Ok(h) => h,
            Err(_) => return None,
        };

        if handle.is_invalid() {
            return None;
        }

        let hglobal = HGLOBAL(handle.0);
        let ptr = GlobalLock(hglobal) as *const u16;
        if ptr.is_null() {
            return None;
        }

        let size = GlobalSize(hglobal);
        let max_len = size / 2;

        let mut len = 0;
        while len < max_len && *ptr.add(len) != 0 {
            len += 1;
        }

        let slice = std::slice::from_raw_parts(ptr, len);
        let text = String::from_utf16_lossy(slice);

        let _ = GlobalUnlock(hglobal);
        Some(text)
    }
}

pub fn write_clipboard_text(text: &str) -> Result<(), String> {
    let _guard = ClipboardGuard::open().ok_or_else(|| "Failed to open clipboard".to_string())?;
    unsafe {
        let _ = EmptyClipboard();

        let utf16: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes_len = utf16.len() * 2;

        let hglobal = GlobalAlloc(GMEM_MOVEABLE, bytes_len)
            .map_err(|e| format!("GlobalAlloc failed: {}", e))?;

        let dest = GlobalLock(hglobal) as *mut u16;
        if dest.is_null() {
            return Err("GlobalLock failed".to_string());
        }

        ptr::copy_nonoverlapping(utf16.as_ptr(), dest, utf16.len());
        let _ = GlobalUnlock(hglobal);

        SetClipboardData(CF_UNICODETEXT, Some(HANDLE(hglobal.0)))
            .map_err(|e| format!("SetClipboardData failed: {}", e))?;

        Ok(())
    }
}

pub fn read_clipboard_bitmap() -> Option<Vec<u8>> {
    let _guard = ClipboardGuard::open()?;
    unsafe {
        if !IsClipboardFormatAvailable(CF_DIB).is_ok() {
            return None;
        }

        let handle = match GetClipboardData(CF_DIB) {
            Ok(h) => h,
            Err(_) => return None,
        };

        if handle.is_invalid() {
            return None;
        }

        let hglobal = HGLOBAL(handle.0);
        let size = GlobalSize(hglobal);
        if size == 0 {
            return None;
        }

        let ptr = GlobalLock(hglobal) as *const u8;
        if ptr.is_null() {
            return None;
        }

        let slice = std::slice::from_raw_parts(ptr, size);
        let data = slice.to_vec();

        let _ = GlobalUnlock(hglobal);
        Some(data)
    }
}

pub fn write_clipboard_bitmap(dib_bytes: &[u8]) -> Result<(), String> {
    let _guard = ClipboardGuard::open().ok_or_else(|| "Failed to open clipboard".to_string())?;
    unsafe {
        let _ = EmptyClipboard();

        let hglobal = GlobalAlloc(GMEM_MOVEABLE, dib_bytes.len())
            .map_err(|e| format!("GlobalAlloc failed: {}", e))?;

        let dest = GlobalLock(hglobal) as *mut u8;
        if dest.is_null() {
            return Err("GlobalLock failed".to_string());
        }

        ptr::copy_nonoverlapping(dib_bytes.as_ptr(), dest, dib_bytes.len());
        let _ = GlobalUnlock(hglobal);

        SetClipboardData(CF_DIB, Some(HANDLE(hglobal.0)))
            .map_err(|e| format!("SetClipboardData failed: {}", e))?;

        Ok(())
    }
}
