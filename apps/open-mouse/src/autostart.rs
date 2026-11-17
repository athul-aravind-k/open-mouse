use std::env;
use windows::core::w;
use windows::Win32::System::Registry::{
    RegCloseKey, RegDeleteValueW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_WRITE, REG_SZ,
};

const RUN_KEY: windows::core::PCWSTR = w!("Software\\Microsoft\\Windows\\CurrentVersion\\Run");
const APP_NAME: windows::core::PCWSTR = w!("OpenMouse");

pub fn is_autostart_enabled() -> bool {
    unsafe {
        let mut hkey = HKEY::default();
        if RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_READ, &mut hkey).is_err() {
            return false;
        }

        let mut data_type = REG_SZ;
        let mut data_len = 0u32;
        let res = RegQueryValueExW(hkey, APP_NAME, None, Some(&mut data_type), None, Some(&mut data_len));
        let _ = RegCloseKey(hkey);

        res.is_ok() && data_len > 0
    }
}

pub fn set_autostart(enable: bool) -> Result<(), String> {
    unsafe {
        let mut hkey = HKEY::default();
        RegOpenKeyExW(HKEY_CURRENT_USER, RUN_KEY, Some(0), KEY_WRITE, &mut hkey)
            .ok()
            .map_err(|e| format!("Failed to open registry key: {}", e))?;

        let res: Result<(), String> = if enable {
            let exe_path = env::current_exe().map_err(|e| e.to_string())?;
            let path_str = format!("\"{}\"\0", exe_path.to_string_lossy());
            let wide: Vec<u16> = path_str.encode_utf16().collect();
            let byte_slice = std::slice::from_raw_parts(
                wide.as_ptr() as *const u8,
                wide.len() * std::mem::size_of::<u16>(),
            );
            RegSetValueExW(hkey, APP_NAME, None, REG_SZ, Some(byte_slice))
                .ok()
                .map_err(|e| format!("RegSetValueExW failed: {}", e))
        } else {
            let _ = RegDeleteValueW(hkey, APP_NAME);
            Ok(())
        };

        let _ = RegCloseKey(hkey);
        res
    }
}
