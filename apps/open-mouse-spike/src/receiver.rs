use capture::{get_cursor_pos, get_primary_screen_size, set_cursor_pos};
use emulate::InputInjector;
use proto::{read_packet_framed, write_packet_framed, Packet};
use std::env;
use std::net::TcpStream;
use std::thread;
use std::time::{Duration, Instant};

fn main() {
    println!("============================================================");
    println!("      OpenMouse - Phase 1 Spike: Client / Receiver          ");
    println!("============================================================");

    let args: Vec<String> = env::args().collect();
    let target_addr = if args.len() > 1 {
        args[1].clone()
    } else {
        "127.0.0.1:48899".to_string()
    };

    println!("[Receiver] Connecting to sender at {}...", target_addr);
    println!("[Receiver] (If this takes more than a few seconds, verify both laptops are on the same Wi-Fi)");

    let mut last_log = Instant::now();
    let stream = loop {
        match TcpStream::connect(&target_addr) {
            Ok(s) => {
                println!("\n[Receiver] *** Connected successfully to sender! ***");
                s.set_nodelay(true).ok();
                break s;
            }
            Err(e) => {
                if last_log.elapsed() >= Duration::from_secs(3) {
                    println!("[Receiver] Waiting for sender at {} (Status: {})...", target_addr, e);
                    last_log = Instant::now();
                }
                thread::sleep(Duration::from_millis(500));
            }
        }
    };

    let mut stream_write = stream.try_clone().expect("Failed to clone TCP stream for writes");
    let mut stream_read = stream;

    let (screen_w, screen_h) = get_primary_screen_size();
    println!("[Receiver] Screen dimensions: {}x{}", screen_w, screen_h);

    let mut injector = InputInjector::new();
    let mut is_controlled = false;
    let mut entry_time = Instant::now();
    let entry_lockout = Duration::from_millis(150);

    loop {
        match read_packet_framed(&mut stream_read) {
            Ok(pkt) => match pkt {
                Packet::Hello {
                    machine_name,
                    version,
                    ..
                } => {
                    println!(
                        "[Receiver] Peer Hello received from {} (version {})",
                        machine_name, version
                    );
                }
                Packet::Enter { x_ratio, y_ratio } => {
                    let target_x = ((x_ratio * screen_w as f32) as i32).max(1);
                    let target_y = (y_ratio * screen_h as f32).clamp(0.0, (screen_h - 1) as f32) as i32;
                    println!(
                        "[Receiver] Entering at ({}, {}) [ratio ({:.2}, {:.2})]",
                        target_x, target_y, x_ratio, y_ratio
                    );
                    set_cursor_pos(target_x, target_y);
                    is_controlled = true;
                    entry_time = Instant::now();
                }
                Packet::Leave { .. } => {
                    println!("[Receiver] Leave event. Releasing held keys/buttons.");
                    injector.release_all();
                    is_controlled = false;
                }
                Packet::MouseMoveRel { dx, dy } => {
                    if is_controlled {
                        let (cur_x, cur_y) = get_cursor_pos();
                        let in_lockout = entry_time.elapsed() < entry_lockout;

                        // Check return boundary (left edge) with 150ms residence lockout guard
                        if (cur_x <= 1 || cur_x + dx <= 0) && dx < 0 && !in_lockout {
                            println!(
                                "[Receiver] Cursor reached return boundary (x: {}, y: {}). Returning to Sender!",
                                cur_x, cur_y
                            );
                            let y_ratio = (cur_y as f32 / screen_h as f32).clamp(0.0, 1.0);
                            let leave_pkt = Packet::Leave {
                                x_ratio: 0.0,
                                y_ratio,
                            };
                            let _ = write_packet_framed(&mut stream_write, &leave_pkt);
                            injector.release_all();
                            is_controlled = false;
                        } else {
                            injector.move_rel(dx, dy);
                        }
                    }
                }
                Packet::MouseButton { button, pressed } => {
                    if is_controlled {
                        injector.mouse_button(button, pressed);
                    }
                }
                Packet::MouseScroll { delta_x, delta_y } => {
                    if is_controlled {
                        injector.mouse_scroll(delta_x, delta_y);
                    }
                }
                Packet::Key {
                    scancode,
                    extended,
                    pressed,
                } => {
                    if is_controlled {
                        injector.key(scancode, extended, pressed);
                    }
                }
                Packet::EmergencyReset => {
                    println!("[Receiver] EmergencyReset received! Releasing all keys/buttons.");
                    injector.release_all();
                    is_controlled = false;
                }
                _ => {}
            },
            Err(e) => {
                eprintln!(
                    "[Receiver] Disconnected: {}. Releasing all inputs and exiting.",
                    e
                );
                injector.release_all();
                break;
            }
        }
    }
}
