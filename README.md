# OpenMouse 🖱️⚡

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue.svg)](https://www.microsoft.com/windows)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green.svg)](Cargo.toml)

**OpenMouse** is a high-performance, low-latency software KVM (Keyboard, Video, Mouse — input and clipboard sharing) written in Rust. Specifically engineered for dual Windows laptop setups, OpenMouse allows you to seamlessly share a single mouse, keyboard, and clipboard across machines over your local Wi-Fi or wired network, featuring smooth edge transitions and drag-and-drop screen topology management.

Zero cloud dependencies, zero telemetry — pure peer-to-peer performance with native Windows Win32 API integration.

---

## Table of Contents

- [Features](#features)
- [Architecture & Multi-Crate Layout](#architecture--multi-crate-layout)
- [Prerequisites](#prerequisites)
- [Building with Cargo](#building-with-cargo)
- [Running the Spikes (Phase 0 / Phase 1)](#running-the-spikes-phase-0--phase-1)
  - [Step 1: Start Host (Sender)](#step-1-start-host-sender)
  - [Step 2: Start Client (Receiver)](#step-2-start-client-receiver)
  - [Edge Transitions & Boundary Behavior](#edge-transitions--boundary-behavior)
- [Safety Escape Hotkey (Ctrl + Alt + Esc)](#safety-escape-hotkey-ctrl--alt--esc)
- [Project Roadmap](#project-roadmap)
- [License](#license)

---

## Features

- **Ultra-Low Latency**: Lightweight binary serialization via [`postcard`](https://crates.io/crates/postcard) over TCP with `TCP_NODELAY` enabled.
- **Hardware-Level Input Capture**: Windows Raw Input (`WM_INPUT`) with `RIDEV_INPUTSINK` on a dedicated message-only window (`HWND_MESSAGE`) delivering high-resolution relative deltas without cursor freeze.
- **Reliable Input Emulation**: Win32 `SendInput` synthesis supporting relative mouse movements, high-resolution mouse wheels, and scancode-based keyboard injection.
- **Infallible Key Tracking**: `KeyTracker` maintains state of all active keys and buttons, executing an automatic `release_all()` safeguard on transition or disconnect to prevent sticky keys.
- **Safety Escape Override**: Hardware-level emergency escape hotkey (`Ctrl + Alt + Esc`) that immediately restores local cursor control under any condition.
- **Bi-directional Edge Handoff**: Hysteresis boundaries and cooldown timers prevent cursor jitter at screen edges.
- **Clipboard Sync** *(Phase 3)*: Win32 clipboard monitoring (`WM_CLIPBOARDUPDATE`) with hash-based deduplication for plain text and images.
- **Peer-to-Peer Security** *(Phase 4)*: Zero cloud servers; direct local LAN communication with end-to-end encryption.

---

## Architecture & Multi-Crate Layout

OpenMouse is designed as a modular Cargo workspace separating input capture, emulation, networking, and daemon logic:

```text
open-mouse/
├── Cargo.toml                  # Workspace manifest and centralized dependencies
├── Cargo.lock                  # Pinned dependency graph for reproducible builds
├── .cargo/
│   └── config.toml             # Target & linker configuration
├── apps/
│   ├── open-mouse-spike/       # Phase 0 & Phase 1 verification binaries
│   │   ├── src/sender.rs       # Host test application (captures & transmits input)
│   │   └── src/receiver.rs     # Client test application (receives & injects input)
│   └── open-mouse-cli/         # OpenMouse CLI and service manager
│       └── src/main.rs
└── crates/
    ├── proto/                  # Wire protocol, packet definitions & postcard serialization
    ├── capture/                # Windows hooks, Raw Input sink & cursor management
    │   ├── src/raw_input.rs    # WM_INPUT sink with RIDEV_INPUTSINK (message-only window)
    │   ├── src/cursor.rs       # Screen metrics, ClipCursor clamping & position helpers
    │   └── src/hook.rs         # Low-level hooks (WH_MOUSE_LL, WH_KEYBOARD_LL) & safety hotkey
    ├── emulate/                # Windows input injection & stuck-key prevention
    │   ├── src/injector.rs     # Win32 SendInput synthesis
    │   └── src/key_tracker.rs  # State tracking and infallible release_all()
    ├── daemon/                 # KVM state machine, edge hysteresis & topology coordinator
    ├── net/                    # Network transport, framing, and connection lifecycle
    └── clipboard/              # Win32 clipboard monitoring and synchronization
```

### Crate Descriptions

| Crate | Responsibilities |
|---|---|
| **`proto`** | Defines `Packet` types (`Hello`, `Enter`, `Leave`, `MouseMoveRel`, `MouseButton`, `MouseScroll`, `Key`, `ClipboardOffer`, `ClipboardRequest`, `ClipboardData`, `EmergencyReset`). Uses `postcard` and length-delimited stream framing. |
| **`capture`** | Captures mouse and keyboard input on Windows. Combines `WH_MOUSE_LL` and `WH_KEYBOARD_LL` hooks for edge crossing detection and input swallowing with `WM_INPUT` (`RIDEV_INPUTSINK`) for hardware mouse deltas while the cursor is clamped with `ClipCursor`. |
| **`emulate`** | Uses Win32 `SendInput` to inject relative mouse movement, buttons, wheels, and keystrokes using hardware scancodes. Includes `KeyTracker` to prevent stuck keys. |
| **`daemon`** | Coordinates KVM states (`Local`, `RemoteActive`, `RemoteControlled`, `Disconnected`), screen layout boundaries, transition cooldowns, and hysteresis margins. |
| **`net`** | Manages TCP/socket transport, framing, heartbeat health checks, and automatic reconnection. |
| **`clipboard`** | Monitors the Windows clipboard via `AddClipboardFormatListener`, computes format hashes to prevent echo loops, and synchronizes text and bitmap data. |
| **`open-mouse-spike`** | Standalone executable binaries (`spike-sender` and `spike-receiver`) for rapid validation of capture, transport, and injection pipelines. |
| **`open-mouse-cli`** | Command-line management tool for configuring screen layouts, managing the daemon, and viewing real-time connection status. |

---

## Prerequisites

- **Operating System**: Windows 10 or Windows 11 (64-bit).
- **Rust Toolchain**: Rust 1.75+ (stable). Install via [rustup.rs](https://rustup.rs/).
- **C Compiler / Linker**:
  - **MSVC (Recommended)**: Visual Studio 2022 or Build Tools with "Desktop development with C++".
  - **MinGW-w64 (GNU)**: GCC toolchain (e.g., via WinLibs or MSYS2) configured for `x86_64-pc-windows-gnu`.
- **Local Network**: Both laptops must be connected to the same local network (Wi-Fi or Ethernet), with TCP port `48899` accessible between the machines.

---

## Building with Cargo

Check the workspace for syntax and type correctness:
```powershell
cargo check --workspace
```

Build all workspace crates and binaries in debug mode:
```powershell
cargo build --workspace
```

Build optimized release binaries (recommended for lowest input latency):
```powershell
cargo build --workspace --release
```

Run unit tests across all crates:
```powershell
cargo test --workspace
```

Check code style and linting:
```powershell
cargo clippy --workspace
```

---

## Running the Spikes (Phase 0 / Phase 1)

The workspace includes interactive spike applications to verify end-to-end input capture, network transfer, and synthetic injection between two Windows laptops.

### Step 1: Start Host (Sender)

On the **Host laptop** (the laptop with the physical mouse and keyboard):

```powershell
# Listen on default address (0.0.0.0:48899)
cargo run -p open-mouse-spike --bin spike-sender

# Or listen on a specific IP address / port:
cargo run -p open-mouse-spike --bin spike-sender -- 0.0.0.0:48899
```

When started, the sender will print its detected primary screen resolution and wait for an incoming connection from the client.

### Step 2: Start Client (Receiver)

On the **Client laptop** (the laptop you want to control):

```powershell
# Connect to host laptop via local LAN IP
cargo run -p open-mouse-spike --bin spike-receiver -- <HOST_IP>:48899

# Example:
cargo run -p open-mouse-spike --bin spike-receiver -- 192.168.1.50:48899
```

*(Note: For single-machine loopback testing, you can omit the IP or specify `127.0.0.1:48899`).*

### Edge Transitions & Boundary Behavior

1. **Host to Remote**:
   - Push your cursor on the Host laptop to the **far right edge** of the primary screen (`x >= screen_width - 2`).
   - The Host sender detects the edge crossing, parks the cursor at the boundary using `ClipCursor`, activates local input swallowing, and transmits `Packet::Enter` with the scaled `y_ratio`.
   - The Client receiver receives `Packet::Enter`, positions its cursor at the corresponding entry point on its screen, and begins injecting mouse movements, clicks, scrolls, and keystrokes.
2. **Remote to Host Return**:
   - Push the cursor on the Client laptop past the **left edge** back towards the Host.
   - The Client transmits `Packet::Leave`, calls `injector.release_all()`, and surrenders control.
   - The Host unclips the cursor (`unclip_cursor()`), deactivates input swallowing, restores local cursor mobility, and activates an entry cooldown to prevent bounceback.

---

## Safety Escape Hotkey (Ctrl + Alt + Esc)

When developing or running software KVM tools, input capture issues or network interruptions could theoretically trap cursor or keyboard control. OpenMouse includes an infallible, hardware-level emergency escape hotkey:

$$\mathbf{Ctrl} + \mathbf{Alt} + \mathbf{Esc}$$

### How It Works:
- **Low-Level Intercept**: Handled inside the Windows low-level keyboard hook procedure (`WH_KEYBOARD_LL`) before any input swallowing or network dispatch occurs.
- **Host Immediate Recovery**:
  1. Instantly clears input swallowing (`set_swallowing(false)`).
  2. Unclips the cursor via `ClipCursor(None)` to restore complete local mouse movement.
  3. Re-centers the cursor to the middle of the screen (`screen_width / 2`, `screen_height / 2`) to prevent accidental immediate re-triggering at the screen edge.
  4. Sends `Packet::EmergencyReset` to the connected client.
- **Client Immediate Recovery**:
  1. Upon receiving `Packet::EmergencyReset` (or upon TCP connection loss), the client immediately executes `injector.release_all()`.
  2. All currently depressed mouse buttons and keys are synthesized as `KEYUP` and `UP` events, preventing stuck modifiers (such as stuck Shift, Ctrl, or Alt).

You can press `Ctrl + Alt + Esc` at any moment to instantly reclaim 100% local control.

---

## Project Roadmap

- [x] **Phase 0: Spikes & Foundation**
  - [x] Cargo workspace setup and architecture definition.
  - [x] Wire protocol (`crates/proto`) with `postcard` serialization and length-delimited framing.
  - [x] Win32 input synthesis (`crates/emulate`) and `KeyTracker` with stuck-key prevention.
  - [x] Low-level hook capture (`WH_MOUSE_LL` / `WH_KEYBOARD_LL`) with edge detection and input swallowing.
  - [x] Hardware emergency escape hotkey (`Ctrl + Alt + Esc`).
  - [x] Standard `.gitignore`, developer documentation, and initial repository commit.
- [ ] **Phase 1: Core KVM & Raw Input**
  - [ ] Windows Raw Input sink (`WM_INPUT`) with `RIDEV_INPUTSINK` and message-only window (`HWND_MESSAGE`).
  - [ ] Cursor parking with hardware relative mickeys (`lLastX`, `lLastY`).
  - [ ] Full bi-directional `Enter` and `Leave` handoff between sender and receiver.
  - [ ] Hysteresis margins and transition cooldown timers.
- [ ] **Phase 2: Topology, Edge Detection & Daemon**
  - [ ] Daemon state machine (`Local`, `RemoteActive`, `RemoteControlled`, `Disconnected`).
  - [ ] Configurable screen arrangements (Left/Right, Top/Bottom, custom offsets).
  - [ ] Multi-monitor support and per-monitor DPI scale compensation.
  - [ ] TCP keepalive, heartbeat pings, and automatic background reconnection.
- [ ] **Phase 3: Clipboard Synchronization**
  - [ ] Win32 clipboard listener (`WM_CLIPBOARDUPDATE`).
  - [ ] SHA-256 hash checking to prevent echo feedback loops.
  - [ ] Text (`CF_UNICODETEXT`) and Bitmap image clipboard synchronization over framed streams.
- [ ] **Phase 4: Encryption, Peer Discovery & Modern GUI**
  - [ ] End-to-end transport encryption (TLS 1.3 or Noise Protocol).
  - [ ] Zero-configuration local network discovery via mDNS / SSDP.
  - [ ] Modern native Windows GUI (Slint or Tauri) with interactive drag-and-drop screen layout canvas.
  - [ ] System tray integration with quick disconnect and settings toggles.

---

## License

This project is licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](http://www.apache.org/licenses/LICENSE-2.0))
- MIT License ([LICENSE-MIT](http://opensource.org/licenses/MIT))

at your option.
