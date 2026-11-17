# OpenMouse

[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)
[![Platform](https://img.shields.io/badge/platform-Windows%2010%20%7C%2011-blue.svg)](https://www.microsoft.com/windows)
[![License](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-green.svg)](Cargo.toml)

OpenMouse is a high-performance, ultra-low-latency software KVM (Keyboard, Video, Mouse input and clipboard sharing) written in Rust. Specifically engineered for dual Windows laptop setups, OpenMouse allows you to seamlessly share a single mouse, keyboard, and clipboard across machines over local Wi-Fi or wired networks, featuring smooth edge transitions, interactive drag-and-drop screen topology management, and comprehensive Windows OS resilience.

Zero cloud dependencies, zero telemetry -- pure peer-to-peer performance with native Windows Win32 API integration, TLS 1.3 encrypted QUIC transport, and hardware-level input capture.

---

## Table of Contents

- [Core Highlights](#core-highlights)
- [Architecture and Multi-Crate Workspace](#architecture-and-multi-crate-workspace)
- [Key Subsystems](#key-subsystems)
  - [Native QUIC Transport and TLS 1.3](#native-quic-transport-and-tls-13)
  - [Zero-Configuration LAN Auto-Discovery](#zero-configuration-lan-auto-discovery)
  - [Hardware-Level Input Capture and Injection](#hardware-level-input-capture-and-injection)
  - [Shared Clipboard (Text and Bitmaps)](#shared-clipboard-text-and-bitmaps)
  - [Visual Screen Arrangement Manager and Identify Overlay](#visual-screen-arrangement-manager-and-identify-overlay)
  - [Emergency Escape and Stuck-Key Prevention](#emergency-escape-and-stuck-key-prevention)
  - [Hardening, Sleep-Wake and Workstation Lock Resilience](#hardening-sleep-wake-and-workstation-lock-resilience)
- [Prerequisites](#prerequisites)
- [Quickstart: Dual-Laptop Setup](#quickstart-dual-laptop-setup)
  - [Step 1: Build Binaries](#step-1-build-binaries)
  - [Step 2: Windows Firewall Configuration](#step-2-windows-firewall-configuration)
  - [Step 3: Launch OpenMouse on Both Laptops](#step-3-launch-openmouse-on-both-laptops)
  - [Step 4: Arrange Screen Topology](#step-4-arrange-screen-topology)
- [Tray Menu Reference](#tray-menu-reference)
- [Verification and Testing](#verification-and-testing)
- [Project Roadmap](#project-roadmap)
- [License](#license)

---

## Core Highlights

- Ultra-Low Latency: Microsecond input delivery leveraging native QUIC unreliable datagrams for high-frequency mouse motion and reliable streams for keystrokes.
- Native QUIC Transport with TLS 1.3: End-to-end encryption via self-signed ECDSA certificates with SHA-256 fingerprint validation and Trust On First Use (TOFU) pinning.
- Zero-Configuration LAN Auto-Discovery: Autonomous UDP beacon broadcast and listener on port 48898 with deterministic tie-breaking.
- Shared Clipboard: Seamless clipboard synchronization for text (UTF-16) and bitmap images up to 10 MB with hash-based loop suppression.
- Interactive Screen Arrangement Canvas: Visual drag-and-drop display topology manager with magnetic edge snapping and on-screen "Identify Displays" identification badges.
- Infallible Hardware Emergency Escape: Dedicated `Ctrl + Alt + Esc` intercept in low-level hooks instantly releases cursor clamping, unswallows input, and clears held keys.
- Windows System Integration: Embedded Per-Monitor DPI Awareness v2 manifest (`open-mouse.exe.manifest`), system startup registry autostart toggle, and sleep/wake (`WM_POWERBROADCAST`) / lock (`WM_WTSSESSION_CHANGE`) event resilience.

---

## Architecture and Multi-Crate Workspace

OpenMouse is structured as a modular Cargo workspace separating input capture, emulation, networking, clipboard synchronization, state management, and user interfaces:

```text
open-mouse/
├── Cargo.toml                  # Workspace manifest and centralized dependencies
├── Cargo.lock                  # Pinned dependency graph
├── .cargo/
│   └── config.toml             # Target and linker configuration (MinGW / MSVC)
├── apps/
│   ├── open-mouse/             # Primary system tray application and GUI
│   │   ├── build.rs            # Manifest compilation via windres or rc.exe
│   │   ├── app.rc              # Windows resource script embedding manifest
│   │   ├── open-mouse.exe.manifest # PerMonitorV2 manifest and OS compatibility GUIDs
│   │   └── src/
│   │       ├── main.rs         # Application entry point, event router, firewall setup
│   │       ├── tray.rs         # System tray window, power/session/display message pump
│   │       ├── layout_window.rs # Visual screen arrangement drag-and-drop canvas
│   │       ├── identify_overlay.rs # Multi-screen display identification overlay
│   │       └── autostart.rs    # Windows Registry Run key autostart manager
│   ├── open-mouse-cli/         # Command-line configuration tool
│   └── open-mouse-spike/       # Protocol and pipeline validation spikes
├── crates/
│   ├── proto/                  # Wire protocol packets and postcard binary serialization
│   ├── capture/                # Low-level hooks (WH_MOUSE_LL, WH_KEYBOARD_LL) and Raw Input
│   ├── emulate/                # Win32 SendInput synthesis and KeyTracker stuck-key safeguards
│   ├── net/                    # Quinn QUIC transport, TLS 1.3 cert generation and pinning
│   ├── clipboard/              # Win32 clipboard listener and text/image sync engine
│   └── daemon/                 # KVM state machine, LAN discovery service and peer coordinator
├── scripts/
│   └── setup-firewall.ps1      # Automated Windows Defender Firewall setup script with UAC
└── docs/
    └── code_review_plan.md     # Comprehensive 5-pillar security and concurrency review plan
```

### Workspace Crate Inventory

| Crate | Responsibilities |
|---|---|
| `proto` | Defines wire `Packet` structures (`Hello`, `Enter`, `Leave`, `MouseMoveRel`, `MouseButton`, `MouseScroll`, `Key`, `ClipboardOffer`, `ClipboardRequest`, `ClipboardData`, `EmergencyReset`, `Identify`, `ArrangementUpdate`, `Heartbeat`). Uses compact `postcard` binary serialization. |
| `capture` | Captures local mouse and keyboard input. Integrates low-level hooks (`WH_MOUSE_LL`, `WH_KEYBOARD_LL`) for boundary edge detection and input swallowing with Windows Raw Input (`WM_INPUT` with `RIDEV_INPUTSINK`) for hardware mouse mickeys. Exposes `rehook()` for sleep/wake recovery. |
| `emulate` | Injects synthetic mouse movements, mouse clicks, mouse wheel scrolls, and keystrokes using Win32 `SendInput`. Tracks depressed keys in `KeyTracker` with an infallible `release_all()` safeguard. |
| `net` | Manages native QUIC transport over UDP port 48899 via `quinn` and `rustls`. Implements self-signed certificate generation, fingerprint verification, and multiplexing across unreliable datagrams and reliable streams. |
| `clipboard` | Intercepts Windows clipboard changes via `AddClipboardFormatListener` and `WM_CLIPBOARDUPDATE`. Computes SHA-256 hashes to suppress circular echo loops. Serializes and deserializes UTF-16 text and DIB/BMP images up to 10 MB. |
| `daemon` | Houses `KvmStateMachine` for coordinate tracking, edge crossing detection, return coordinate calculation, and hysteresis cooldown. Runs `DiscoveryService` for LAN UDP broadcast beacons on port 48898 and coordinates peer lifecycle in `PeerNode`. |
| `open-mouse` | Main Windows tray application. Provides hidden broadcast message handling (`WM_POWERBROADCAST`, `WM_WTSSESSION_CHANGE`, `WM_DISPLAYCHANGE`), visual monitor arrangement canvas, display identification overlays, and firewall launch. |

---

## Key Subsystems

### Native QUIC Transport and TLS 1.3

OpenMouse utilizes native QUIC (over UDP port 48899) implemented via `quinn` and `rustls`:
- Multiplexing: High-frequency mouse delta packets are transmitted over unreliable QUIC datagrams, guaranteeing zero head-of-line blocking and minimal latency. Key events, clipboard streams, and arrangement updates travel over reliable, ordered QUIC streams.
- Security: All traffic is encrypted using TLS 1.3. Each node generates an ephemeral ECDSA P-256 certificate on first boot and computes its SHA-256 fingerprint. Peer certificates are pinned using a Trust On First Use (TOFU) policy stored in `config.json`.
- Health Monitoring: The transport runs continuous round-trip time (RTT) tracking and heartbeat checks, automatically initiating key release and fallback to searching if connection loss occurs.

### Zero-Configuration LAN Auto-Discovery

`DiscoveryService` operates on UDP port 48898:
- Autonomous Beacons: Nodes broadcast periodic discovery beacons containing machine identifier, device name, and listening port.
- Deterministic Tie-Breaking: When two peers detect each other simultaneously, a deterministic lexicographical tie-breaker (`local_id < remote_id`) designates one machine as connector and the other as listener, preventing dual-connection contention.
- Instant Wake Broadcast: Upon system resume from sleep, `broadcast_now()` sends immediate beacons without waiting for the default timer interval.

### Hardware-Level Input Capture and Injection

Input handling is engineered for sub-millisecond responsiveness without cursor drift:
- Windows Raw Input: A message-only window (`HWND_MESSAGE`) registers for mouse input via `RegisterRawInputDevices` with `RIDEV_INPUTSINK`. Relative mickeys (`lLastX`, `lLastY`) are read directly from hardware packets, avoiding Windows mouse acceleration distortion.
- Cursor Clamping: When control transitions to the remote peer, the cursor is parked at the boundary and clamped using Win32 `ClipCursor` to prevent accidental clicks on background local applications.
- Low-Level Hooks: `WH_MOUSE_LL` and `WH_KEYBOARD_LL` hooks intercept edge triggers and swallow local keystrokes when remote control is active, routing input exclusively to the remote peer.

### Shared Clipboard (Text and Bitmaps)

OpenMouse synchronizes clipboard content transparently between connected machines:
- Format Support: Supports standard UTF-16 Unicode plain text (`CF_UNICODETEXT`) and bitmap images (`CF_DIBV5` / `CF_DIB`) up to 10 MB.
- Loop Suppression: The clipboard engine computes a cryptographic hash of clipboard contents. When remote clipboard data is injected locally, its hash is recorded in a suppression cache to prevent ping-pong reflection back across the network.
- Stream Transfer: Large clipboard payloads are transmitted asynchronously over dedicated QUIC streams without delaying high-priority mouse and keyboard packets.

### Visual Screen Arrangement Manager and Identify Overlay

Clicking "Arrange Displays..." from the tray opens the Screen Arrangement Manager:
- Drag-and-Drop Canvas: Displays a visual representation of both laptops. Drag the remote screen around the local screen to align them matching your physical desk configuration.
- Magnetic Edge Snapping: Screens automatically snap to Right, Left, Top, or Bottom boundaries when moved within threshold proximity.
- Identify Displays: Clicking "Identify Displays" flashes full-screen semi-transparent overlay badges ("1" on local, "2" on remote) for 2.5 seconds, ensuring accurate physical-to-virtual mapping.

### Emergency Escape and Stuck-Key Prevention

Safety mechanisms prevent trapped cursors or sticky modifier keys:
- Emergency Hotkey (`Ctrl + Alt + Esc`): Handled directly inside `kbd_proc` before any swallowing logic. Instantly releases `ClipCursor`, halts input swallowing, recenters the cursor on the primary screen, and notifies the peer.
- Stuck-Key Safeguard: `KeyTracker` records all keydown and buttondown events. On any state transition, emergency escape, network disconnect, or workstation lock, `injector.release_all()` synthesizes matching `KEYUP` and `UP` events for all currently depressed inputs.

### Hardening, Sleep-Wake and Workstation Lock Resilience

OpenMouse is hardened against Windows power state and session transitions:
- Sleep / Wake (`WM_POWERBROADCAST`):
  - On `PBT_APMSUSPEND`: Unclips cursor, releases all keys via `injector.release_all()`, returns state machine to `Local`, and notifies the remote peer.
  - On `PBT_APMRESUMEAUTOMATIC` / `PBT_APMRESUMESUSPEND`: Calls `CaptureEngine::rehook()` (`WM_USER + 201`) to reinstall clean low-level hooks, unclips cursor, and triggers immediate UDP discovery broadcast via `broadcast_now()`.
- Workstation Lock (`WM_WTSSESSION_CHANGE`):
  - On `WTS_SESSION_LOCK`: Unclips the cursor and releases all keys so local credentials can be entered on the secure lock desktop without interference.
  - On `WTS_SESSION_UNLOCK`: Re-arms capture hooks and restores operational state.
- Screen Topology Updates (`WM_DISPLAYCHANGE`):
  - Dynamically extracts updated monitor dimensions, updates `KvmStateMachine::set_local_screen_size`, and transmits an updated `Packet::Hello` with updated `ScreenDescriptor` to the connected peer.

---

## Prerequisites

- Operating System: Windows 10 or Windows 11 (64-bit).
- Rust Toolchain: Rust 1.75+ (stable). Install via [rustup.rs](https://rustup.rs/).
- C Compiler / Linker:
  - MSVC (Recommended): Visual Studio 2022 or Build Tools with "Desktop development with C++".
  - MinGW-w64 (GNU): GCC toolchain (e.g. WinLibs or MSYS2) configured for `x86_64-pc-windows-gnu`.
- Local Network: Both laptops connected to the same Wi-Fi network or Ethernet subnet.

---

## Quickstart: Dual-Laptop Setup

Follow these steps to connect two Windows laptops:

### Step 1: Build Binaries

Clone the repository and build the release executable on both machines:

```powershell
git clone https://github.com/athul/open-mouse.git
cd open-mouse
cargo build --release --bin open-mouse
```

The compiled executable is located at `target\release\open-mouse.exe`.

### Step 2: Windows Firewall Configuration

OpenMouse requires inbound UDP access on ports 48898 (discovery) and 48899 (QUIC). You can configure the Windows Firewall using either method:

Method A: Run the Automated PowerShell Script
```powershell
powershell -ExecutionPolicy Bypass -File scripts\setup-firewall.ps1
```
The script automatically requests administrator elevation (UAC) and registers inbound rules across Domain, Private, and Public profiles.

Method B: Tray Menu Option
Launch `open-mouse.exe`, right-click the system tray icon, and click **Configure Firewall...**. Accept the Windows UAC elevation prompt.

### Step 3: Launch OpenMouse on Both Laptops

Run `open-mouse.exe` on Laptop A and Laptop B:

```powershell
.\target\release\open-mouse.exe
```

1. OpenMouse creates a tray icon and begins broadcasting LAN discovery beacons on UDP 48898.
2. Within seconds, the laptops discover each other and establish an encrypted QUIC session on UDP 48899.
3. The tray icon status updates from "Searching for peers on LAN" to "Status: Connected to <PeerName> (<RTT> ms)".

### Step 4: Arrange Screen Topology

1. Right-click the OpenMouse tray icon and select **Arrange Displays...**.
2. Click **Identify Displays** to verify which machine is display 1 and display 2.
3. Drag the remote display box to match your desk layout (e.g. place it to the right of your primary screen).
4. Click **Apply Layout**.
5. Move your mouse across the boundary edge -- your cursor seamlessly transitions to the second laptop!

---

## Tray Menu Reference

Right-clicking the OpenMouse system tray icon displays the following context menu:

- Status Header: Displays current connection state, peer name, and round-trip latency (read-only).
- Arrange Displays...: Opens the interactive visual monitor arrangement canvas.
- Edit Config (Peer IP & Settings): Opens `config.json` in Notepad for manual configuration.
- Switch Shared Edge: Toggles the transition boundary between Right, Left, Top, and Bottom edges.
- Emergency Unlock (Ctrl+Alt+Esc): Immediately releases cursor clipping, clears held keys, and restores local control.
- Start with Windows: Toggles automatic launch at Windows login via the CurrentUser Run registry key.
- Configure Firewall...: Executes `scripts/setup-firewall.ps1` with UAC elevation.
- Exit OpenMouse: Safely unhooks capture pipelines, restores cursor mobility, and terminates the application.

---

## Verification and Testing

Execute the complete test suite across the workspace:

```powershell
# Run all unit and integration tests across all workspace crates
cargo test --workspace

# Compile optimized release binary
cargo build --release --bin open-mouse

# Validate code formatting and clippy lints
cargo clippy --workspace
```

To verify the embedded Per-Monitor DPI Awareness v2 application manifest in the compiled binary:

```powershell
powershell -Command "$bytes = [System.IO.File]::ReadAllBytes('target\release\open-mouse.exe'); $text = [System.Text.Encoding]::UTF8.GetString($bytes); Write-Host 'PerMonitorV2 Present:' $text.Contains('PerMonitorV2'); Write-Host 'Win10/11 GUID Present:' $text.Contains('8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a')"
```

---

## Project Roadmap

- [x] Phase 0: Spikes and Foundation
  - [x] Cargo workspace setup and architecture definition.
  - [x] Wire protocol (`crates/proto`) with `postcard` serialization and length-delimited framing.
  - [x] Win32 input synthesis (`crates/emulate`) and `KeyTracker` stuck-key safeguards.
  - [x] Low-level hook capture (`WH_MOUSE_LL` / `WH_KEYBOARD_LL`) with edge detection and input swallowing.
  - [x] Hardware emergency escape hotkey (`Ctrl + Alt + Esc`).
- [x] Phase 1: Core KVM and Raw Input
  - [x] Windows Raw Input sink (`WM_INPUT`) with `RIDEV_INPUTSINK` on message-only window (`HWND_MESSAGE`).
  - [x] Cursor parking with hardware relative mickeys (`lLastX`, `lLastY`).
  - [x] Full bi-directional `Enter` and `Leave` handoff between sender and receiver.
  - [x] Hysteresis margins and transition cooldown timers.
- [x] Phase 2: Topology, Edge Detection and Daemon
  - [x] Daemon state machine (`Local`, `RemoteActive`, `RemoteControlled`, `Disconnected`).
  - [x] Configurable screen arrangements (Left, Right, Top, Bottom, and continuous offsets).
  - [x] Interactive screen arrangement window (`layout_window.rs`) with magnetic edge snapping.
  - [x] Multi-display "Identify Displays" on-screen overlay (`identify_overlay.rs`).
- [x] Phase 3: Clipboard Synchronization
  - [x] Win32 clipboard monitoring via `AddClipboardFormatListener` and `WM_CLIPBOARDUPDATE`.
  - [x] SHA-256 hash checking to prevent echo feedback loops.
  - [x] Text (`CF_UNICODETEXT`) and Bitmap image (`CF_DIBV5`) clipboard synchronization over framed streams up to 10 MB.
- [x] Phase 4: Encryption, Peer Discovery and Modern GUI
  - [x] Native QUIC transport (`crates/net`) with TLS 1.3 encryption and TOFU certificate fingerprint pinning.
  - [x] Zero-configuration local network discovery (`crates/daemon/src/discovery.rs`) on UDP port 48898.
  - [x] Native Windows system tray application (`apps/open-mouse`) with interactive context menus.
  - [x] Windows startup autostart integration via registry `HKCU\Software\Microsoft\Windows\CurrentVersion\Run`.
- [x] Phase 5: Hardening, Resilience and Windows Integration
  - [x] Sleep / wake resilience (`WM_POWERBROADCAST`: `PBT_APMSUSPEND` and `PBT_APMRESUME...`).
  - [x] Workstation lock resilience (`WM_WTSSESSION_CHANGE` via `WTSRegisterSessionNotification`).
  - [x] Screen resolution dynamic adaptation (`WM_DISPLAYCHANGE`).
  - [x] Dynamic hook lifecycle re-hooking (`CaptureEngine::rehook` via `WM_USER + 201`).
  - [x] Native Windows manifest declaring `PerMonitorV2` DPI awareness and Windows 10/11 compatibility GUIDs.
  - [x] Automated Windows Defender Firewall script (`scripts/setup-firewall.ps1`) and tray menu option.
  - [x] Strict zero-emoji repository compliance and comprehensive code review plan.

---

## License

This project is licensed under either of:

- Apache License, Version 2.0 ([LICENSE-APACHE](http://www.apache.org/licenses/LICENSE-2.0))
- MIT License ([LICENSE-MIT](http://opensource.org/licenses/MIT))

at your option.
