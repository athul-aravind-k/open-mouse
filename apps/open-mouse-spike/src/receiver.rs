use capture::{get_primary_screen_size, set_cursor_pos};
use emulate::InputInjector;
use proto::{read_packet_framed, Packet};
use std::env;
use std::net::TcpStream;
use std::thread;
use std::time::Duration;

fn main() {
    println!("============================================================");
    println!("      OpenMouse - Phase 0 Spike: Client / Receiver          ");
    println!("============================================================");

    let args: Vec<String> = env::args().collect();
    let target_addr = if args.len() > 1 {
        args[1].clone()
    } else {
        "127.0.0.1:48899".to_string()
    };

    println!("[Receiver] Connecting to sender at {}...", target_addr);

    let mut stream = loop {
        match TcpStream::connect(&target_addr) {
            Ok(s) => {
                println!("[Receiver] Connected successfully to sender!");
                s.set_nodelay(true).ok();
                break s;
            }
            Err(_) => {
                thread::sleep(Duration::from_millis(500));
            }
        }
    };

    let (screen_w, screen_h) = get_primary_screen_size();
    println!("[Receiver] Screen dimensions: {}x{}", screen_w, screen_h);

    let mut injector = InputInjector::new();

    loop {
        match read_packet_framed(&mut stream) {
            Ok(pkt) => match pkt {
                Packet::Hello { machine_name, version, .. } => {
                    println!("[Receiver] Peer Hello received from {} (version {})", machine_name, version);
                }
                Packet::Enter { x_ratio, y_ratio } => {
                    let target_x = (x_ratio * screen_w as f32) as i32;
                    let target_y = (y_ratio * screen_h as f32) as i32;
                    println!("[Receiver] Entering at ({}, {}) [ratio ({:.2}, {:.2})]", target_x, target_y, x_ratio, y_ratio);
                    set_cursor_pos(target_x, target_y);
                }
                Packet::Leave { .. } => {
                    println!("[Receiver] Leave event. Releasing held keys/buttons.");
                    injector.release_all();
                }
                Packet::MouseMoveRel { dx, dy } => {
                    injector.move_rel(dx, dy);
                }
                Packet::MouseButton { button, pressed } => {
                    injector.mouse_button(button, pressed);
                }
                Packet::MouseScroll { delta_x, delta_y } => {
                    injector.mouse_scroll(delta_x, delta_y);
                }
                Packet::Key { scancode, extended, pressed } => {
                    injector.key(scancode, extended, pressed);
                }
                Packet::EmergencyReset => {
                    println!("[Receiver] EmergencyReset received! Releasing all keys/buttons.");
                    injector.release_all();
                }
                _ => {}
            },
            Err(e) => {
                eprintln!("[Receiver] Disconnected: {}. Releasing all inputs and exiting.", e);
                injector.release_all();
                break;
            }
        }
    }
}
