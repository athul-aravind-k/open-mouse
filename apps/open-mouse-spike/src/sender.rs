use capture::{
    clip_cursor_to_point, get_primary_screen_size, set_cursor_pos, unclip_cursor,
    CaptureEngine, CapturedEvent,
};
use proto::{write_packet_framed, Packet};
use std::env;
use std::net::TcpListener;

fn main() {
    println!("============================================================");
    println!("      OpenMouse - Phase 0 Spike: Host / Sender              ");
    println!("============================================================");
    println!("Safety: Press Ctrl + Alt + Esc at ANY time to recover control!");
    println!("------------------------------------------------------------");

    let args: Vec<String> = env::args().collect();
    let addr = if args.len() > 1 {
        args[1].clone()
    } else {
        "0.0.0.0:48899".to_string()
    };

    println!("[Sender] Listening for client connection on {}...", addr);
    let listener = TcpListener::bind(&addr).unwrap_or_else(|e| {
        panic!("Failed to bind to {}: {}", addr, e);
    });

    let (mut stream, peer_addr) = listener.accept().expect("Failed to accept connection");
    stream.set_nodelay(true).ok();
    println!("[Sender] Connected to receiver at {}", peer_addr);

    let (screen_w, screen_h) = get_primary_screen_size();
    println!("[Sender] Primary screen resolution: {}x{}", screen_w, screen_h);
    println!("[Sender] Instructions: Move cursor to the RIGHT edge (x >= {}) to enter remote screen.", screen_w - 2);

    let (capture, rx) = CaptureEngine::new();
    let mut is_remote = false;

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
        },
    };
    write_packet_framed(&mut stream, &hello).ok();

    while let Ok(event) = rx.recv() {
        match event {
            CapturedEvent::EdgeTrigger { x, y } => {
                if !is_remote && x >= screen_w - 2 {
                    println!("[Sender] Cursor reached RIGHT border (x: {}, y: {}). Transferring to remote!", x, y);
                    is_remote = true;
                    capture.set_swallowing(true);
                    clip_cursor_to_point(screen_w - 1, y);

                    let y_ratio = (y as f32 / screen_h as f32).clamp(0.0, 1.0);
                    let enter_pkt = Packet::Enter {
                        x_ratio: 0.0,
                        y_ratio,
                    };
                    if let Err(e) = write_packet_framed(&mut stream, &enter_pkt) {
                        eprintln!("[Sender] Error sending Enter: {}. Releasing...", e);
                        is_remote = false;
                        capture.set_swallowing(false);
                        unclip_cursor();
                    }
                }
            }
            CapturedEvent::Packet(pkt) => {
                if is_remote {
                    if let Err(e) = write_packet_framed(&mut stream, &pkt) {
                        eprintln!("[Sender] Network write error: {}. Restoring local control.", e);
                        is_remote = false;
                        capture.set_swallowing(false);
                        unclip_cursor();
                    }
                }
            }
            CapturedEvent::EmergencyEscape => {
                println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
                println!("!!! [EMERGENCY ESCAPE] Ctrl+Alt+Esc triggered!           !!!");
                println!("!!! Releasing all input hooks and restoring cursor.      !!!");
                println!("!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!!");
                is_remote = false;
                capture.set_swallowing(false);
                unclip_cursor();
                set_cursor_pos(screen_w / 2, screen_h / 2);
                let _ = write_packet_framed(&mut stream, &Packet::EmergencyReset);
            }
        }
    }
}
