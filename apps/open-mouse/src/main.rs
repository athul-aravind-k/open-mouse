#![windows_subsystem = "windows"]

mod autostart;
mod identify_overlay;
mod layout_window;
mod tray;

use daemon::{get_config_path, load_config, save_config, PeerNode, PeerStatus};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tray::{
    run_message_loop, TrayEvent, TrayManager, AUTOSTART_ENABLED, CURRENT_EDGE_TEXT,
    CURRENT_STATUS_TEXT, ID_ARRANGE_DISPLAYS, ID_AUTOSTART, ID_CONFIG_EDIT, ID_EXIT,
    ID_SETUP_FIREWALL, ID_SWITCH_EDGE, ID_UNLOCK,
};
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

fn main() {
    // Enable Per-Monitor DPI Awareness v2
    unsafe {
        let _ = windows::Win32::UI::HiDpi::SetProcessDpiAwarenessContext(
            windows::Win32::UI::HiDpi::DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        );
    }

    let config = load_config();
    let autostart_active = autostart::is_autostart_enabled();

    *CURRENT_STATUS_TEXT.lock().unwrap() = Some("Status: Initializing...".to_string());
    *CURRENT_EDGE_TEXT.lock().unwrap() = Some(format!("Shared Edge: {:?}", config.shared_edge));
    *AUTOSTART_ENABLED.lock().unwrap() = autostart_active;

    let running = Arc::new(AtomicBool::new(true));
    let peer_node = Arc::new(PeerNode::new(config.clone()));

    let peer_node_cb = peer_node.clone();
    let running_cb = running.clone();

    let mut tray_mgr = TrayManager::new("OpenMouse - Software KVM", move |event| {
        match event {
            TrayEvent::Command(cmd_id) => match cmd_id {
                ID_EXIT => {
                    running_cb.store(false, Ordering::SeqCst);
                    peer_node_cb.stop();
                    unsafe {
                        PostQuitMessage(0);
                    }
                }
                ID_ARRANGE_DISPLAYS => {
                    layout_window::open_layout_window(peer_node_cb.clone());
                }
                ID_CONFIG_EDIT => {
                    let path = get_config_path();
                    let _ = Command::new("notepad.exe").arg(path).spawn();
                }
                ID_UNLOCK => {
                    // Emergency unlock - forces unswallow & reset
                    capture::unclip_cursor();
                    emulate::InputInjector::new().release_all();
                }
                ID_SWITCH_EDGE => {
                    let new_edge = peer_node_cb.toggle_edge();
                    *CURRENT_EDGE_TEXT.lock().unwrap() = Some(format!("Shared Edge: {:?}", new_edge));
                }
                ID_AUTOSTART => {
                    let current = autostart::is_autostart_enabled();
                    let new_state = !current;
                    let _ = autostart::set_autostart(new_state);
                    let mut cfg = load_config();
                    cfg.autostart = new_state;
                    let _ = save_config(&cfg);
                    *AUTOSTART_ENABLED.lock().unwrap() = new_state;
                }
                ID_SETUP_FIREWALL => {
                    launch_firewall_setup();
                }
                _ => {}
            },
            TrayEvent::PowerSuspend => {
                tracing::info!("System suspend received, notifying peer node");
                peer_node_cb.notify_power_suspend();
            }
            TrayEvent::PowerResume => {
                tracing::info!("System resume received, notifying peer node");
                peer_node_cb.notify_power_resume();
            }
            TrayEvent::SessionLock => {
                tracing::info!("Workstation session locked, releasing input");
                peer_node_cb.notify_power_suspend();
            }
            TrayEvent::SessionUnlock => {
                tracing::info!("Workstation session unlocked, restoring input");
                peer_node_cb.notify_power_resume();
            }
            TrayEvent::DisplayChange { width, height } => {
                tracing::info!("Display topology changed to {}x{}", width, height);
                peer_node_cb.notify_display_change(width, height);
            }
        }
    });

    tray_mgr.show_balloon("OpenMouse Running", "OpenMouse is active in your system tray.");

    // Background thread to listen to on-screen identify requests
    let identify_rx = peer_node.identify_receiver();
    let running_identify = running.clone();
    thread::spawn(move || {
        while running_identify.load(Ordering::SeqCst) {
            if let Ok((num, duration_ms)) = identify_rx.recv() {
                identify_overlay::show_identify_overlay(num, duration_ms);
            }
        }
    });

    // Background thread to listen to PeerNode status updates and refresh the tray icon
    let status_rx = peer_node.status_receiver();
    let running_status = running.clone();
    thread::spawn(move || {
        while running_status.load(Ordering::SeqCst) {
            if let Ok(st) = status_rx.recv() {
                let status_text = match st {
                    PeerStatus::SearchingOnLan { port } => {
                        format!("Status: Searching for peers on LAN (Port {})", port)
                    }
                    PeerStatus::WaitingForPeer { local_ip, port } => {
                        format!("Status: Listening on {}:{}", local_ip, port)
                    }
                    PeerStatus::Connecting { target } => {
                        format!("Status: Connecting to {}", target)
                    }
                    PeerStatus::Connected { peer_name, rtt_ms, .. } => {
                        if let Some(rtt) = rtt_ms {
                            format!("Status: Connected to {} ({:.1} ms)", peer_name, rtt)
                        } else {
                            format!("Status: Connected to {}", peer_name)
                        }
                    }
                    PeerStatus::ControllingRemote => {
                        "Status: Controlling Remote Screen".to_string()
                    }
                    PeerStatus::RemoteControlled => {
                        "Status: Remote Controlled".to_string()
                    }
                };

                *CURRENT_STATUS_TEXT.lock().unwrap() = Some(status_text);
            }
        }
    });

    run_message_loop(running);
}

fn find_firewall_script() -> Option<std::path::PathBuf> {
    let candidates = [
        std::path::PathBuf::from("scripts").join("setup-firewall.ps1"),
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|dir| dir.join("scripts").join("setup-firewall.ps1")))
            .unwrap_or_default(),
        std::env::current_exe()
            .ok()
            .and_then(|p| {
                p.parent()
                    .and_then(|p| p.parent())
                    .map(|dir| dir.join("scripts").join("setup-firewall.ps1"))
            })
            .unwrap_or_default(),
    ];

    candidates.into_iter().find(|p| p.exists())
}

fn launch_firewall_setup() {
    let script_path = find_firewall_script().unwrap_or_else(|| std::path::PathBuf::from("scripts/setup-firewall.ps1"));

    use windows::core::w;
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let op = w!("runas");
    let file = w!("powershell.exe");
    let arg_str = format!(
        "-NoProfile -ExecutionPolicy Bypass -File \"{}\"",
        script_path.display()
    );
    let params: Vec<u16> = arg_str.encode_utf16().chain(std::iter::once(0)).collect();

    unsafe {
        let res = ShellExecuteW(
            None,
            op,
            file,
            windows::core::PCWSTR(params.as_ptr()),
            None,
            SW_SHOWNORMAL,
        );
        if res.0 as usize <= 32 {
            tracing::warn!("Firewall setup elevation was cancelled or failed (code: {:?})", res.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;
    use tray::{
        ID_ARRANGE_DISPLAYS, ID_AUTOSTART, ID_CONFIG_EDIT, ID_EXIT, ID_SETUP_FIREWALL,
        ID_STATUS_HEADER, ID_SWITCH_EDGE, ID_UNLOCK,
    };

    #[test]
    fn test_tray_menu_ids_unique() {
        let ids = [
            ID_STATUS_HEADER,
            ID_CONFIG_EDIT,
            ID_SWITCH_EDGE,
            ID_UNLOCK,
            ID_AUTOSTART,
            ID_EXIT,
            ID_ARRANGE_DISPLAYS,
            ID_SETUP_FIREWALL,
        ];
        let set: HashSet<_> = ids.iter().collect();
        assert_eq!(set.len(), ids.len(), "Tray menu IDs must be unique");
    }

    #[test]
    fn test_tray_event_variants() {
        let cmd = TrayEvent::Command(2008);
        assert_eq!(cmd, TrayEvent::Command(ID_SETUP_FIREWALL));

        let suspend = TrayEvent::PowerSuspend;
        let resume = TrayEvent::PowerResume;
        let lock = TrayEvent::SessionLock;
        let unlock = TrayEvent::SessionUnlock;
        let disp = TrayEvent::DisplayChange {
            width: 1920,
            height: 1080,
        };

        assert_ne!(suspend, resume);
        assert_ne!(lock, unlock);
        match disp {
            TrayEvent::DisplayChange { width, height } => {
                assert_eq!(width, 1920);
                assert_eq!(height, 1080);
            }
            _ => panic!("Expected DisplayChange"),
        }
    }

    #[test]
    fn test_find_firewall_script_search() {
        let _ = find_firewall_script();
    }
}
