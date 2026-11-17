# OpenMouse Comprehensive Code Review Plan

## 1. Executive Summary and Scope

OpenMouse is a high-performance, low-latency software KVM bridging low-level Windows kernel/Win32 APIs (Raw Input, low-level hooks, GDI, ClipCursor, Terminal Services) with modern asynchronous networking (Tokio, Quinn QUIC, Rustls TLS 1.3).

This Code Review Plan establishes a rigorous verification framework across the entire codebase to evaluate safety, robustness, security boundaries, and latency constraints. The plan covers all crates and applications in the repository:
- `crates/proto`: Wire protocol serialization, framing, and packet models.
- `crates/capture`: Low-level hooks, Raw Input message pumps, and cursor management.
- `crates/emulate`: Win32 synthetic injection and KeyTracker state machine.
- `crates/net`: Quinn QUIC transport, TLS 1.3 certificate generation, and TOFU verification.
- `crates/clipboard`: Win32 clipboard monitoring, hash deduplication, and streaming.
- `crates/daemon`: KVM state machine, LAN UDP discovery, and peer lifecycle.
- `apps/open-mouse`: System tray application, DPI awareness manifest, layout window, overlay, and OS lifecycle integration.

---

## 2. Pillar 1: Security and Cryptography

### 2.1 Threat Model and Attack Surface
- Inbound UDP Port 48898 (Discovery): Unauthenticated UDP broadcast beacon listener. Vulnerable to malformed beacon floods, packet spoofing, and amplification attacks.
- Inbound UDP Port 48899 (QUIC Transport): Authenticated TLS 1.3 QUIC connection endpoint. Protected by cryptographic handshake, but exposed to denial-of-service via resource exhaustion before handshake completion.
- Local IPC / Win32 Clipboard: Ingestion of arbitrary text and image payloads from local applications and remote peers.

### 2.2 TLS 1.3 and Certificate Verification
- Review Focus:
  - Verify that `crates/net/src/cert.rs` generates self-signed certificates with cryptographic hygiene (ECDSA P-256 / SHA-256).
  - Verify that `crates/net/src/verifier.rs` enforces certificate fingerprint pinning according to the Trust On First Use (TOFU) policy.
  - Ensure untrusted peer certificates cannot silently bypass validation or hijack active sessions.
  - Audit certificate expiration and re-generation handling.

### 2.3 Packet Framing and Allocation Limits
- Review Focus:
  - Verify strict upper bounds on all network stream decoders:
    - Maximum stream packet frame size: 32 MB limit.
    - Maximum frame buffer limit: 64 MB cap.
    - Maximum clipboard payload: 10 MB cap.
  - Ensure that receiving a malformed packet length header cannot trigger out-of-memory (OOM) panics or unbounded vector allocations.
  - Verify `postcard` binary deserialization limits on high-frequency datagrams.

### 2.4 Input Sanitization and Boundary Validation
- Review Focus:
  - Validate that received scancodes, mouse coordinates, and button identifiers in `crates/emulate/src/injector.rs` are within valid Win32 ranges.
  - Ensure remote peers cannot inject privileged Windows key sequences (e.g. Win+L, Ctrl+Alt+Del) or escape sandboxing.
  - Confirm that injected coordinate inputs clamp cleanly to primary and virtual screen bounds.

---

## 3. Pillar 2: Win32 API Safety and Resource Lifecycle

### 3.1 GDI Object Lifecycle and Leak Auditing
- Review Focus:
  - Audit all GDI drawing routines in `apps/open-mouse/src/layout_window.rs` and `apps/open-mouse/src/identify_overlay.rs`.
  - Verify that every call to `CreateSolidBrush`, `CreateFontW`, `CreateCompatibleDC`, and `CreateCompatibleBitmap` is strictly matched with a corresponding `DeleteObject` or `DeleteDC`.
  - Verify that the previous GDI object returned by `SelectObject` is always restored before deleting the temporary object or DC.
  - Confirm that `BeginPaint` is unconditionally paired with `EndPaint`, and `GetDC` with `ReleaseDC`.
  - Validate that high-frequency repaints (e.g. dragging displays on canvas) maintain flat GDI handle counts in Windows Task Manager.

### 3.2 Low-Level Hook Cleanup and Re-Hooking Stability
- Review Focus:
  - Verify that `WH_MOUSE_LL` and `WH_KEYBOARD_LL` hooks in `crates/capture/src/hook.rs` are unconditionally removed via `UnhookWindowsHookEx` on thread termination or `CaptureEngine::drop`.
  - Verify that `CaptureEngine::rehook()` (`WM_USER + 201`) cleanly tears down existing hooks before reinstalling fresh hooks after system sleep.
  - Confirm that the hook thread message pump processes `WM_REHOOK` directly within `GetMessageW` without routing `NULL` window handles to `DispatchMessageW`.
  - Verify that `HOOK_STATE` mutex is never held across Win32 API calls (`UnhookWindowsHookEx`, `SetWindowsHookExW`) to eliminate deadlocks.

### 3.3 ClipCursor Safety Guarantees
- Review Focus:
  - Audit all transition paths in `crates/daemon/src/peer.rs` and `crates/capture/src/cursor.rs`.
  - Confirm that `ClipCursor(None)` is unconditionally executed on:
    1. Transition from `RemoteActive` back to `Local`.
    2. Emergency escape (`Ctrl + Alt + Esc`).
    3. Workstation lock (`WTS_SESSION_LOCK`).
    4. System sleep (`PBT_APMSUSPEND`).
    5. Network disconnection (`NodeEvent::NetworkDisconnected`).
    6. Application exit (`ID_EXIT` and `Drop`).
  - Validate that `CursorClipGuard` utilizes RAII drop semantics so that thread panics never leave the local user cursor permanently clamped.

### 3.4 Win32 Clipboard Memory Semantics
- Review Focus:
  - Verify memory ownership rules in `crates/clipboard/src/manager.rs`.
  - When writing to the clipboard via `SetClipboardData`:
    - Confirm memory is allocated using `GlobalAlloc(GMEM_MOVEABLE, ...)`.
    - Confirm memory is locked with `GlobalLock`, populated, and unlocked with `GlobalUnlock`.
    - Verify that once `SetClipboardData` succeeds, the system owns the memory handle and the application does NOT call `GlobalFree`.
    - Verify that if `SetClipboardData` fails, the allocated handle is freed immediately to prevent memory leaks.
  - Verify that `OpenClipboard` is always paired with `CloseClipboard`, even when intermediate format operations fail.

### 3.5 Terminal Services Notification Cleanup
- Review Focus:
  - Verify that `WTSRegisterSessionNotification` in `apps/open-mouse/src/tray.rs` is paired with `WTSUnRegisterSessionNotification` on both `WM_DESTROY` and `TrayManager::drop`.

---

## 4. Pillar 3: Concurrency, Threading and Synchronization

### 4.1 Async Runtime and Win32 STA UI Isolation
- Review Focus:
  - Verify strict separation between Tokio multi-threaded asynchronous tasks and Win32 Single-Threaded Apartment (STA) UI message pumps.
  - Confirm that no Win32 GUI or Hook message pump (`GetMessageW` / `DispatchMessageW`) runs inside a Tokio worker thread.
  - Confirm that Tokio runtime calls do not block Win32 UI threads, ensuring tray responsiveness and fluid mouse tracking.

### 4.2 Lock Ordering and Deadlock Prevention
- Review Focus:
  - Audit mutex acquisition order across all shared states:
    - `PeerNode::config` (`Arc<Mutex<AppConfig>>`)
    - `PeerNode::status` (`Arc<Mutex<PeerStatus>>`)
    - `PeerNode::remote_screen` (`Arc<Mutex<Option<ScreenDescriptor>>>`)
    - `CURRENT_STATUS_TEXT`, `CURRENT_EDGE_TEXT`, `AUTOSTART_ENABLED`
  - Verify that no mutex guard is held across `.await` suspension points.
  - Verify that locks are held for the minimal necessary duration and released before channel sends or network transmission.

### 4.3 Channel Flow Control and Backpressure
- Review Focus:
  - Inspect `crossbeam_channel` usage across capture, peer lifecycle, and network layers.
  - Confirm that high-frequency mouse deltas (up to 1000 Hz) do not cause unbounded channel queue growth during temporary network stalls.
  - Validate that `recv_timeout(100ms)` in `run_peer_lifecycle` allows regular checking of the `running` atomic flag without thread starvation.

---

## 5. Pillar 4: Latency, Throughput and Performance Optimization

### 5.1 Zero-Allocation High-Frequency Serialization
- Review Focus:
  - Verify that `Packet::MouseMoveRel`, `Packet::MouseButton`, and `Packet::MouseScroll` serialize to fixed-size buffers using `postcard` without heap allocations (`Vec` / `String`).
  - Confirm that datagram payload sizes stay well under network MTU (typically under 64 bytes for mouse packets) to eliminate UDP fragmentation.

### 5.2 Windows Raw Input Mickeys
- Review Focus:
  - Verify that mouse input is captured via `WM_INPUT` with `RIDEV_INPUTSINK` in `crates/capture/src/raw_input.rs`.
  - Confirm that hardware deltas (`lLastX`, `lLastY`) bypass Windows Enhanced Pointer Precision (ballistics curves) for 1:1 hardware translation.
  - Ensure zero input lag between physical mouse displacement and UDP datagram dispatch.

### 5.3 QUIC Datagram vs Stream Multiplexing
- Review Focus:
  - Verify that high-frequency mouse deltas are transmitted over unreliable QUIC datagrams (`send_datagram`), preventing Head-of-Line (HoL) blocking when packets drop.
  - Verify that critical control packets (`Hello`, `Enter`, `Leave`, `Key`, `Clipboard`) travel over reliable streams (`open_bi` / `open_uni`).

### 5.4 Screen Arrangement Rendering Efficiency
- Review Focus:
  - Confirm double-buffering via memory DC in `layout_window.rs` to eliminate screen flicker during display drag-and-drop.
  - Verify that coordinate calculation during dragging involves only lightweight integer arithmetic.

---

## 6. Pillar 5: Edge Cases, Resilience and Error Recovery

### 6.1 Power State Transitions (Sleep, Wake, Modern Standby)
- Review Focus:
  - Validate that `WM_POWERBROADCAST` messages (`PBT_APMSUSPEND`, `PBT_APMRESUMEAUTOMATIC`, `PBT_APMRESUMESUSPEND`) are intercepted by `apps/open-mouse/src/tray.rs`.
  - On Suspend: Confirm immediate cursor unclip, key release via `KeyTracker::release_all()`, state machine return to `Local`, and `Packet::EmergencyReset` notification to peer.
  - On Resume: Confirm low-level hook re-registration (`CaptureEngine::rehook()`), cursor unclip safety assertion, and immediate discovery broadcast (`DiscoveryService::broadcast_now()`).
  - Validate idempotency: rapid suspend/resume bursts must not cause duplicate hook allocations or crash the daemon.

### 6.2 Workstation Lock and Desktop Switches
- Review Focus:
  - Validate `WM_WTSSESSION_CHANGE` handling for `WTS_SESSION_LOCK` and `WTS_SESSION_UNLOCK`.
  - When the workstation locks (e.g. Win+L), confirm immediate release of cursor clamping and key release, ensuring the user can freely interact with the Windows credential/login prompt.

### 6.3 Dynamic Screen Topology Adaptation
- Review Focus:
  - Validate `WM_DISPLAYCHANGE` handling in `tray.rs` and `peer.rs`.
  - Confirm that screen resolution changes (docking/undocking, resolution adjustment) invoke `KvmStateMachine::set_local_screen_size` and transmit an updated `Packet::Hello` with new `ScreenDescriptor`.
  - Ensure that transient 0x0 display metrics during monitor handshake are ignored.

### 6.4 Sudden Network Loss and Reconnection
- Review Focus:
  - Verify that Quinn idle timeout (configured to 8 seconds) and keepalive interval (1 second) detect abrupt connection loss (e.g. Wi-Fi drop or laptop lid close).
  - Verify that upon disconnection, `NodeEvent::NetworkDisconnected` resets the state machine to `Local`, clears swallowing, unclips the cursor, and releases all keys.
  - Confirm that `DiscoveryService` resumes broadcasting and automatically reconnects once network connectivity returns.

### 6.5 Panic Recovery and Termination Hygiene
- Review Focus:
  - Ensure that if any worker thread panics, `ClipCursor(None)` and `injector.release_all()` are executed to prevent trapping user input.
  - Verify that `apps/open-mouse/src/main.rs` exits cleanly on `WM_QUIT` and `ID_EXIT`.

---

## 7. Audit Checklist and Verification Matrix

| Area | Check Item | Severity | Automated Test / Command | Verified |
|---|---|---|---|---|
| **Security** | TLS 1.3 self-signed cert generation & SHA-256 fingerprinting | Critical | `cargo test -p net` | Yes |
| **Security** | TOFU certificate pinning & untrusted peer rejection | Critical | `cargo test -p net` | Yes |
| **Security** | Stream framing upper limits (32 MB packet, 64 MB frame, 10 MB clipboard) | High | `cargo test -p proto` | Yes |
| **Security** | Postcard deserialization bounds on unreliable datagrams | High | `cargo test -p proto` | Yes |
| **Win32 Safety** | Low-level hook cleanup on thread termination and drop | Critical | `cargo test -p capture` | Yes |
| **Win32 Safety** | Hook lifecycle re-hooking via WM_REHOOK (WM_USER + 201) | Critical | `cargo test -p capture` | Yes |
| **Win32 Safety** | Cursor unclip guarantees on switch, emergency escape, lock, sleep | Critical | `cargo test -p capture` | Yes |
| **Win32 Safety** | KeyTracker infallible release_all() on disconnect & state changes | Critical | `cargo test -p emulate` | Yes |
| **Win32 Safety** | GDI handle leak audit in layout_window and identify_overlay | High | Code inspection & Task Manager | Yes |
| **Win32 Safety** | Win32 Clipboard memory ownership (GMEM_MOVEABLE and GlobalFree) | High | `cargo test -p clipboard` | Yes |
| **Win32 Safety** | WTSRegisterSessionNotification & WTSUnRegisterSessionNotification | Medium | Code inspection | Yes |
| **Concurrency** | Complete isolation between Tokio runtime and Win32 UI message pumps | High | Code inspection | Yes |
| **Concurrency** | Zero mutex locks held across .await suspension points | High | Code inspection | Yes |
| **Concurrency** | Mutex hierarchy preventing deadlocks in peer lifecycle | Medium | `cargo test -p daemon` | Yes |
| **Performance** | Zero-allocation postcard serialization for mouse input packets | High | Code inspection | Yes |
| **Performance** | Raw Input RIDEV_INPUTSINK relative mickeys without acceleration | High | `cargo test -p capture` | Yes |
| **Performance** | Unreliable QUIC datagrams for mouse delta delivery (zero HoL blocking) | High | `cargo test -p net` | Yes |
| **Resilience** | Sleep/wake handling (WM_POWERBROADCAST: suspend & resume) | Critical | `cargo test -p open-mouse` | Yes |
| **Resilience** | Workstation lock resilience (WM_WTSSESSION_CHANGE: WTS_SESSION_LOCK) | Critical | `cargo test -p open-mouse` | Yes |
| **Resilience** | Display topology dynamic adaptation (WM_DISPLAYCHANGE) | High | `cargo test -p daemon` | Yes |
| **Resilience** | PerMonitorV2 manifest embedded in release PE header | High | PE header string check | Yes |
| **Compliance** | Strict zero-emoji compliance across code, docs, scripts, menus | High | UTF-8 code point scan | Yes |

---

## 8. Verification Commands

To perform full verification of this Code Review Plan:

```powershell
# 1. Workspace compilation and unit tests
cargo test --workspace

# 2. Release executable build
cargo build --release --bin open-mouse

# 3. Clippy lint verification
cargo clippy --workspace

# 4. Manifest PE header verification
powershell -Command "$bytes = [System.IO.File]::ReadAllBytes('target\release\open-mouse.exe'); $text = [System.Text.Encoding]::UTF8.GetString($bytes); Write-Host 'PerMonitorV2:' $text.Contains('PerMonitorV2'); Write-Host 'Win10 GUID:' $text.Contains('8e0f7a12-bfb3-4fe8-b9a5-48fd50a15a9a')"

# 5. PowerShell firewall script syntax verification
powershell -NoProfile -Command "& { $s = Get-Content 'scripts\setup-firewall.ps1' -Raw; $errs = $null; [System.Management.Automation.PSParser]::Tokenize($s, [ref]$errs); if ($errs) { throw $errs } else { Write-Host 'Firewall script syntax OK' } }"

# 6. Repository-wide zero emoji audit
powershell -NoProfile -Command "& { $files = Get-ChildItem -Path . -Recurse -File | Where-Object { $_.FullName -notmatch '\\target\\' -and $_.FullName -notmatch '\\\.git\\' -and $_.FullName -notmatch '\\\.agents\\' }; foreach ($f in $files) { $content = [System.IO.File]::ReadAllText($f.FullName); for ($i = 0; $i -lt $content.Length; $i++) { $cp = [char]::ConvertToUtf32($content, $i); if ([char]::IsSurrogatePair($content, $i)) { $i++ }; if (($cp -ge 0x1F300 -and $cp -le 0x1F9FF) -or ($cp -ge 0x2600 -and $cp -le 0x26FF) -or ($cp -ge 0x2700 -and $cp -le 0x27BF) -or ($cp -ge 0x1FA70 -and $cp -le 0x1FAFF) -or ($cp -ge 0x2B50 -and $cp -le 0x2B55) -or ($cp -ge 0xFE00 -and $cp -le 0xFE0F)) { Write-Error ('Emoji at ' + $f.FullName + ':' + $i); exit 1 } } }; Write-Host 'Zero emojis verified across repository.' }"
```
