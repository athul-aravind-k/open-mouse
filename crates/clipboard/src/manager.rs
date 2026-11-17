use crate::windows_clipboard::{
    read_clipboard_bitmap, read_clipboard_text, write_clipboard_bitmap, write_clipboard_text,
};
use proto::ClipFormat;
use std::collections::VecDeque;
use std::hash::{DefaultHasher, Hash, Hasher};

pub const MAX_CLIPBOARD_BYTES: usize = 10 * 1024 * 1024; // 10 MB

pub struct ClipboardManager {
    suppression_cache: VecDeque<u64>,
    max_cache_size: usize,
}

impl Default for ClipboardManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardManager {
    pub fn new() -> Self {
        Self {
            suppression_cache: VecDeque::with_capacity(32),
            max_cache_size: 32,
        }
    }

    pub fn record_remote_hash(&mut self, hash: u64) {
        if self.suppression_cache.len() >= self.max_cache_size {
            self.suppression_cache.pop_front();
        }
        self.suppression_cache.push_back(hash);
    }

    pub fn is_suppressed(&self, hash: u64) -> bool {
        self.suppression_cache.contains(&hash)
    }

    /// Reads current local clipboard, returns payload and hash if it's new and not suppressed.
    pub fn poll_local_change(&mut self) -> Option<(ClipFormat, u64, Vec<u8>)> {
        // Try reading text first
        if let Some(text) = read_clipboard_text() {
            if text.is_empty() {
                return None;
            }
            let bytes = text.into_bytes();
            if bytes.len() > MAX_CLIPBOARD_BYTES {
                return None;
            }
            let hash = compute_hash(ClipFormat::PlainText, &bytes);
            if !self.is_suppressed(hash) {
                self.record_remote_hash(hash);
                tracing::debug!("Local text clipboard captured: {} bytes", bytes.len());
                return Some((ClipFormat::PlainText, hash, bytes));
            }
            return None;
        }

        // Try reading bitmap/image
        if let Some(bitmap) = read_clipboard_bitmap() {
            if bitmap.is_empty() || bitmap.len() > MAX_CLIPBOARD_BYTES {
                return None;
            }
            let hash = compute_hash(ClipFormat::Bitmap, &bitmap);
            if !self.is_suppressed(hash) {
                self.record_remote_hash(hash);
                tracing::debug!("Local bitmap clipboard captured: {} bytes", bitmap.len());
                return Some((ClipFormat::Bitmap, hash, bitmap));
            }
            return None;
        }

        None
    }

    /// Injects remote clipboard data into local Windows clipboard while suppressing reflection.
    pub fn inject_remote(&mut self, format: ClipFormat, hash: u64, payload: &[u8]) -> Result<(), String> {
        if payload.len() > MAX_CLIPBOARD_BYTES {
            return Err("Payload exceeds 10MB limit".to_string());
        }

        // Record hash before writing to OS clipboard so the resulting WM_CLIPBOARDUPDATE is suppressed
        self.record_remote_hash(hash);
        tracing::debug!("Injecting remote clipboard: format={:?}, size={}", format, payload.len());

        match format {
            ClipFormat::PlainText => {
                let text = String::from_utf8(payload.to_vec())
                    .map_err(|e| format!("Invalid UTF-8: {}", e))?;
                write_clipboard_text(&text)?;
            }
            ClipFormat::Bitmap => {
                write_clipboard_bitmap(payload)?;
            }
        }

        Ok(())
    }
}

pub fn compute_hash(format: ClipFormat, data: &[u8]) -> u64 {
    let mut hasher = DefaultHasher::new();
    format.hash(&mut hasher);
    data.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_suppression_cache() {
        let mut mgr = ClipboardManager::new();
        let hash = 123456789;

        assert!(!mgr.is_suppressed(hash));
        mgr.record_remote_hash(hash);
        assert!(mgr.is_suppressed(hash));
    }

    #[test]
    fn test_compute_hash_reproducibility() {
        let data = b"Hello, OpenMouse Clipboard!";
        let h1 = compute_hash(ClipFormat::PlainText, data);
        let h2 = compute_hash(ClipFormat::PlainText, data);
        assert_eq!(h1, h2);

        let h_bmp = compute_hash(ClipFormat::Bitmap, data);
        assert_ne!(h1, h_bmp);
    }
}
