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
    run_message_loop, TrayManager, AUTOSTART_ENABLED, CURRENT_EDGE_TEXT, CURRENT_STATUS_TEXT,
    ID_ARRANGE_DISPLAYS, ID_AUTOSTART, ID_CONFIG_EDIT, ID_EXIT, ID_SWITCH_EDGE, ID_UNLOCK,
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

    let mut tray_mgr = TrayManager::new("OpenMouse - Software KVM", move |cmd_id| {
        match cmd_id {
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
            _ => {}
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
                    PeerStatus::Connected { peer_name, .. } => {
                        format!("Status: Connected to {}", peer_name)
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
