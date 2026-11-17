#![windows_subsystem = "windows"]

mod tray;

use daemon::{get_config_path, load_config, save_config, PeerNode, PeerStatus};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tray::{
    run_message_loop, TrayManager, AUTOSTART_ENABLED, CURRENT_EDGE_TEXT, CURRENT_STATUS_TEXT,
    ID_AUTOSTART, ID_CONFIG_EDIT, ID_EXIT, ID_SWITCH_EDGE, ID_UNLOCK,
};
use windows::Win32::UI::WindowsAndMessaging::PostQuitMessage;

fn main() {
    let config = load_config();
    let autostart = config.autostart;

    *CURRENT_STATUS_TEXT.lock().unwrap() = Some("Status: Initializing...".to_string());
    *CURRENT_EDGE_TEXT.lock().unwrap() = Some(format!("Shared Edge: {:?}", config.shared_edge));
    *AUTOSTART_ENABLED.lock().unwrap() = autostart;

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
                let mut cfg = load_config();
                cfg.autostart = !cfg.autostart;
                let _ = save_config(&cfg);
                *AUTOSTART_ENABLED.lock().unwrap() = cfg.autostart;
            }
            _ => {}
        }
    });

    tray_mgr.show_balloon("OpenMouse Running", "OpenMouse is active in your system tray.");

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
