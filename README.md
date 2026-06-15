<div align="center">

# ⚙️ Project Arbiter

**A deterministic system orchestration and automation engine for Windows.**  
Silent. Stateful. Strictly bounded.

[![CI](https://github.com/Sid-352/Project-Arbiter/actions/workflows/ci.yml/badge.svg)](https://github.com/Sid-352/Project-Arbiter/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sid-352/Project-Arbiter?style=flat-square&logo=github&color=4f46e5&label=release)](https://github.com/Sid-352/Project-Arbiter/releases/latest)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue?style=flat-square)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Stars](https://img.shields.io/github/stars/Sid-352/Project-Arbiter?style=flat-square&logo=github)](https://github.com/Sid-352/Project-Arbiter/stargazers)
[![Contributors](https://img.shields.io/github/contributors/Sid-352/Project-Arbiter?style=flat-square)](https://github.com/Sid-352/Project-Arbiter/graphs/contributors)

</div>

---

## 🧭 What is Arbiter?

Arbiter is a **headless background automation service** built for Windows. It runs silently in the system tray and executes physical and system-level workflows — reliably, without touching the terminal.

> Built to replace flaky Bash scripts, unreliable Task Scheduler jobs, and constant AHK breakage — with something that actually holds state.

---

## 💡 Core Philosophy

| Principle | Description |
|-----------|-------------|
| 🔁 **D-FSM** | Actions follow rigid, explicitly defined Finite State Machines. Execution paths are strictly bounded — no surprises. |
| 🔇 **Headless by Default** | Arbiter runs silently in the tray. File hooks, hotkey triggers, and hardware queues work independently of any UI. |
| 🔒 **Security First** | Every disk op, shell call, and hardware input is gated by hard-coded guards. No unauthorized actions, ever. |

---

## 🏗️ Architecture

Arbiter is split into **four isolated crates**, each with a single responsibility.  
Check out the [Detailed Documentation (Wiki)](https://github.com/Sid-352/Project-Arbiter/wiki) for more.

<details>
<summary><b>🧠 arbiter-core — Logic & State</b></summary>
<br>

Handles all logical state, permissions, configurations, and signal observation. Provides data contracts but executes no instructions.

- **Vigil** — Pluggable observation listeners for hotkeys and file monitoring
- **Atlas** — FSM evaluation loop that maps triggers to action sequences
- **Signet** — Secure configuration vault protected by Windows DPAPI, serialized via MessagePack
- **Filter** — In-memory path lock state to prevent infinite event loops

</details>

<details>
<summary><b>🔌 arbiter-bridge — Hardware & File Execution</b></summary>
<br>

Single-responsibility execution layer. Processes incoming logical directives through a global queuing lock.

- **Runner** — Background orchestration task with a Hibernation Guard
- **Hardware Bridge** — Keyboard/mouse routing with coordinate bounds checks
- **Filesystem Bridge** — Secure IO manager using `PathBuf` for cross-platform safety
- **Shell Bridge** — Hardened subprocess launcher for independent executions

</details>

<details>
<summary><b>🚀 arbiter-app — Entrypoint & Lifecycle</b></summary>
<br>

Entrypoint wrapper managing lifecycle state, custom daily rolling loggers, Tokio async runtime initialization, and system-tray integration.

</details>

<details>
<summary><b>🖥️ arbiter-forge — Visual Interface</b></summary>
<br>

Slint-based GUI for monitoring live telemetry and managing engine state. Connects to the host via a **Named Pipe IPC** protocol.

</details>

---

## 🛡️ Safety & Fallbacks

Arbiter is designed to never operate beyond user-defined constraints.

> [!WARNING]
> Security boundaries are **hard-coded** into the execution pipeline. Unauthorized paths or binaries will result in an error — not a silent skip.

| Guard | What it does |
|-------|-------------|
| 🔒 **Jail Guard** | Clamps all disk operations to a whitelist of trusted root paths |
| ⚙️ **Execution Guard** | Strictly bounds shell/process execution to a pre-calculated whitelist |
| 🖱️ **Hardware Guard** | Enforces coordinate constraints within known monitor dimensions |
| 🔄 **Steady State Filter** | Ignores filesystem events triggered by Arbiter itself |
| 🤝 **Interference Guard** | Detects human presence and enforces a grace period to prevent collisions |
| 🔓 **Hardware Reset Guard** | Releases all keys automatically if the engine terminates unexpectedly |

---

## 📦 Installation

> [!NOTE]
> Arbiter uses a low-level Win32 API (`WH_KEYBOARD_LL`) to capture global hotkeys. Pre-compiled binaries **may be flagged by Windows Defender heuristics**. See options below for a friction-free experience.

**Requirements:**
- 🪟 Windows 10 or later
- 🦀 Rust 1.70+ (for building from source)

### ⚡ Download via PowerShell *(recommended — bypasses SmartScreen)*

```powershell
Invoke-WebRequest -Uri "https://github.com/Sid-352/Project-Arbiter/releases/latest/download/arbiter-windows.zip" -OutFile "arbiter.zip"; Expand-Archive "arbiter.zip" -DestinationPath ".\arbiter"; Unblock-File -Path ".\arbiter\*.exe"
```

### 📥 Download Pre-built Binaries

1. Go to the [Releases page](https://github.com/Sid-352/Project-Arbiter/releases/latest)
2. Extract the downloaded zip
3. Run as Administrator:
   ```powershell
   .\arbiter.exe
   ```

### 📦 Install via Cargo *(guarantees no SmartScreen issues)*

```bash
cargo install --git https://github.com/Sid-352/Project-Arbiter.git arbiter-app arbiter-forge
```

### 🔨 Build from Source

```bash
# Clone the repo
git clone https://github.com/Sid-352/Project-Arbiter.git
cd Project-Arbiter

# Build both binaries
cargo build --release --package arbiter-app
cargo build --release --package arbiter-forge

# Run as Administrator
.\target\release\arbiter.exe
```

---

## 🚀 Quick Start

```
1. Run arbiter.exe        (Admin recommended)
2. Wait for the tray icon to appear
3. Click "Open Forge" from the tray menu
4. Create/save a decree in Forge
5. Drop a matching file into your monitored folder to trigger it
```

> Forge is intended to be launched by Arbiter App from the tray menu.

---

## 🖥️ Usage

**Run the background service:**
```bash
cargo run --release --package arbiter-app
```

**Run the UI** *(Arbiter App must already be running)*:
```bash
cargo run --release --package arbiter-forge
```

---

## 🗺️ Roadmap

- [ ] 🔀 Conditional logic in Decree sequence editor (branching steps from analytical ward data)
- [ ] 🔬 Enhanced Perception: deep-tissue file inspection gates (MIME type, SHA-256)
- [ ] 🔐 Signet vault full passphrase protection via AES-GCM key derivation

---

## 🤝 Contributing

Contributions are welcome! Please read [CONTRIBUTING.md](CONTRIBUTING.md) before submitting a PR.  
See [CODE_OF_CONDUCT.MD](CODE_OF_CONDUCT.MD) for community guidelines.

---

## 🔐 Security

Found a vulnerability? Check [SECURITY.md](SECURITY.md) for responsible disclosure guidelines.

---

## 📄 License

Distributed under the [MIT License](LICENSE). © 2026 Sid-352 & Contributors.

---

<div align="center">

**Built with 🦀 Rust &nbsp;·&nbsp; Powered by [Slint](https://slint.dev/) &nbsp;·&nbsp; Running silently on Windows**

[![Forks](https://img.shields.io/github/forks/Sid-352/Project-Arbiter?style=flat-square)](https://github.com/Sid-352/Project-Arbiter/network/members)
[![Issues](https://img.shields.io/github/issues/Sid-352/Project-Arbiter?style=flat-square)](https://github.com/Sid-352/Project-Arbiter/issues)
[![Last Commit](https://img.shields.io/github/last-commit/Sid-352/Project-Arbiter?style=flat-square)](https://github.com/Sid-352/Project-Arbiter/commits/main)

</div>
