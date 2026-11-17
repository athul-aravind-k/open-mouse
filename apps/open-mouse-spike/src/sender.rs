use capture::{
    get_primary_screen_size, park_cursor, set_cursor_pos, unclip_cursor,
    CaptureEngine, CapturedEvent,
};
use proto::{read_packet_framed, write_packet_framed, Packet};
use std::env;
use std::net::TcpListener;
use std::thread;
use std::time::{Duration, Instant};

enum SenderEvent {
    Captured(CapturedEvent),
    Network(Packet),
    Disconnected,
}

fn main() {
    println!("============================================================");
    println!("      OpenMouse - Phase 1 Spike: Host / Sender              ");
    println!("============================================================");
    println!("Safety: Press Ctrl + Alt + Esc at ANY time to recover control!");
    println!("------------------------------------------------------------");

    let args: Vec<String> = env::args().collect();
    let addr = if args.len() > 1 {
        args[1].clone()
    } else {
        "0.0.0.0:48899".to_string()
    };

    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| {
        panic!("Failed to bind to {}: {}", addr, e);
    });

    println!("[Sender] Listening on 0.0.0.0:48899");
    println!("[Sender] >>> RUN THIS ON YOUR SECOND LAPTOP: <<<");
    println!("           .\\spike-receiver.exe 192.168.1.39:48899");
    println!("------------------------------------------------------------");

    let (screen_w, screen_h) = get_primary_screen_size();
    println!("[Sender] Primary screen resolution: {}x{}", screen_w, screen_h);
    println!(
        "[Sender] Move cursor to RIGHT edge (x >= {}) to transfer to remote.",
        screen_w - 2
    );

    let (capture, rx) = CaptureEngine::new();
    let rx_arc = std::sync::Arc::new(crossbeam_channel::Receiver::clone(&rx));

    loop {
        println!("\n[Sender] Waiting for client to connect...");
        let (stream, peer_addr) = match listener.accept() {
            Ok(conn) => conn,
            Err(e) => {
                eprintln!("[Sender] Accept error: {}. Retrying in 1s...", e);
                thread::sleep(Duration::from_secs(1));
                continue;
            }
        };

        stream.set_nodelay(true).ok();
        println!("[Sender] *** CLIENT CONNECTED from {}! ***", peer_addr);

        let mut stream_write = match stream.try_clone() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[Sender] Failed to clone stream: {}", e);
                continue;
            }
        };
        let mut stream_read = stream;

        // Send initial Hello packet
        let hello = Packet::Hello {
            version: 1,
            machine_name: "Host-A".to_string(),
            screen: proto::ScreenDescriptor {
                machine_id: "Host-A".to_string(),
                width_px: screen_w as u32,
                height_px: screen_h as u32,
                dpi_scale: 1.0,
                offset_x: 0,
                offset_y: 0,
                edge: 1,
            },
            cert_fingerprint: String::new(),
        };
        if let Err(e) = write_packet_framed(&mut stream_write, &hello) {
            eprintln!("[Sender] Failed to send Hello: {}. Resetting connection.", e);
            continue;
        }

        let (event_tx, event_rx) = crossbeam_channel::unbounded();

        // Forward local capture events
        let tx_cap = event_tx.clone();
        let rx_cap = rx_arc.clone();
        let cap_thread = thread::spawn(move || {
            while let Ok(evt) = rx_cap.recv() {
                if tx_cap.send(SenderEvent::Captured(evt)).is_err() {
                    break;
                }
            }
        });

        // Listen for bidirectional network packets from receiver
        let tx_net = event_tx.clone();
        let net_thread = thread::spawn(move || {
            loop {
                match read_packet_framed(&mut stream_read) {
                    Ok(pkt) => {
                        if tx_net.send(SenderEvent::Network(pkt)).is_err() {
                            break;
                        }
                    }
                    Err(_) => {
                        let _ = tx_net.send(SenderEvent::Disconnected);
                        break;
                    }
                }
            }
        });

        let mut is_remote = false;
        let mut cooldown_until = Instant::now();

        while let Ok(event) = event_rx.recv() {
            match event {
                SenderEvent::Captured(CapturedEvent::EdgeTrigger { x, y }) => {
                    if !is_remote && x >= screen_w - 2 && Instant::now() >= cooldown_until {
                        println!(
                            "[Sender] Cursor reached RIGHT border (x: {}, y: {}). Transferring to remote!",
                            x, y
                        );
                        is_remote = true;
                        capture.set_swallowing(true);
                        park_cursor(screen_w - 1, y);

                        let y_ratio = (y as f32 / screen_h as f32).clamp(0.0, 1.0);
                        let enter_pkt = Packet::Enter {
                            x_ratio: 0.0,
                            y_ratio,
                        };
                        if let Err(e) = write_packet_framed(&mut stream_write, &enter_pkt) {
                            eprintln!("[Sender] Error sending Enter: {}. Releasing...", e);
                            is_remote = false;
                            capture.set_swallowing(false);
                            unclip_cursor();
                        }
                    }
                }

                SenderEvent::Captured(CapturedEvent::Packet(pkt)) => {
                    if is_remote {
                        if let Err(e) = write_packet_framed(&mut stream_write, &pkt) {
                            eprintln!("[Sender] Network write error: {}. Restoring local control.", e);
                            is_remote = false;
                            capture.set_swallowing(false);
                            unclip_cursor();
                        }
                    }
                }

                SenderEvent::Captured(CapturedEvent::EmergencyEscape) => {
                    println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
                    println!("!!! [EMERGENCY ESCAPE] Ctrl+Alt+Esc triggered!           !!!");
                    println!("!!! Releasing all input hooks and restoring cursor.      !!!");
                    println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
                    is_remote = false;
                    capture.set_swallowing(false);
                    unclip_cursor();
                    set_cursor_pos(screen_w / 2, screen_h / 2);
                    let _ = write_packet_framed(&mut stream_write, &Packet::EmergencyReset);
                }

                SenderEvent::Network(Packet::Leave { y_ratio, .. }) => {
                    if is_remote {
                        println!(
                            "[Sender] Cursor returned from remote screen (Leave ratio: {:.2})! Restoring local control.",
                            y_ratio
                        );
                        is_remote = false;
                        capture.set_swallowing(false);
                        unclip_cursor();

                        let return_x = (screen_w - 10).max(0);
                        let return_y = (y_ratio * screen_h as f32).clamp(0.0, (screen_h - 1) as f32) as i32;
                        set_cursor_pos(return_x, return_y);
                        cooldown_until = Instant::now() + Duration::from_millis(300);
                    }
                }

                SenderEvent::Network(Packet::EmergencyReset) => {
                    if is_remote {
                        println!("[Sender] EmergencyReset received from peer. Restoring local control.");
                        is_remote = false;
                        capture.set_swallowing(false);
                        unclip_cursor();
                        set_cursor_pos(screen_w / 2, screen_h / 2);
                    }
                }

                SenderEvent::Disconnected => {
                    println!("[Sender] Client disconnected. Returning to listen loop.");
                    if is_remote {
                        capture.set_swallowing(false);
                        unclip_cursor();
                        set_cursor_pos(screen_w / 2, screen_h / 2);
                    }
                    break;
                }

                _ => {}
            }
        }

        // Clean up threads and prepare for next connection
        drop(event_rx);
        let _ = net_thread.join();
        let _ = cap_thread;
    }
}
