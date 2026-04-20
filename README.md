# Project Arbiter

[![CI](https://github.com/Sid-352/Project-Vassal/actions/workflows/ci.yml/badge.svg)](https://github.com/Sid-352/Project-Vassal/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/Sid-352/Project-Vassal?label=release)](https://github.com/Sid-352/Project-Vassal/releases/latest)

Arbiter is a deterministic system orchestration and automation engine. It acts as a silent background service designed to perform physical and system-level workflows reliably. It prioritizes security, stability, and protection against unbounded behavior. I made it to more or less execute scripts that I don't wish to open the terminal for, to arrange my downloads and to perform other repetitive tasks.

## Why Arbiter?
In my experience, simple Bash scripts and Task Scheduler sometimes fail to provide the necessary hardware context or stateful evaluation required for complex workflows. Also I have had had issues with AHK a lot of times. 

## Core Philosophy

* **D-FSM**: Actions follow rigid and explicitly defined FSMs such that execution paths and procedures are strictly bounded.
* **Headless by default**: Arbiter operates primarily as a silent background tray application. File system hooks, hotkey triggers, and hardware queues function independently of a visual interface.

## Architecture

Check out the [Detailed Documentation (Wiki)](https://github.com/Sid-352/Project-Arbiter/wiki) for more information.

Arbiter is split into four seperated component crates to isolate scope.

### 1. arbiter-core
Handles all logical state, permissions, configurations, and signal observation. It provides data contracts but executes no instructions.
* **Vigil**: Pluggable observation listeners for hotkeys and file monitoring.
* **Atlas**: The Finite State Machine evaluation loop that maps triggers to sequences.
* **Signet**: Secure configuration vault managing trusted paths and command whitelists.
* **Filter**: In-memory path lock state that prevents infinite event observation loops.

### 2. arbiter-bridge
A single-responsibility hardware and file execution layer. It processes incoming logical directives through a global queuing lock.
* **Runner**: Background orchestration task that manages sequential action execution.
* **Hardware Bridge**: Physical keyboard and mouse routing handler with coordinate bounds checks.
* **Filesystem Bridge**: Secure file system IO manager handling localized file manipulation using `PathBuf` for cross-platform safety.
* **Shell Bridge**: Hardened sub-process launching utility handling independent executions.

### 3. arbiter-app
Entrypoint wrapper managing lifecycle state, custom daily rolling loggers, Tokio asynchronous runtime initialization, and system-tray integration.

### 4. arbiter-forge
Slint-based visual interface for monitoring live telemetry and managing engine state. It connects to the host via high-performance Named Pipe IPC.

## Safety and Fallbacks

Arbiter is pretty much prevented by design from operating beyond user defined constraints. 

> [!WARNING]
> Security Boundaries are hard-coded into the engine execution pipeline. Failure to authorize paths or binaries will result in an error.

1. **Jail Guard**: All disk operations are clamped to a user-defined whitelist of trusted root paths.
2. **Execution Guard**: Arbitrary shell and process executions are strictly bounded by a pre-calculated whitelist.
3. **Hardware Guard**: Coordinate constraints enforce bounding pointer logic within known monitor dimensions.
4. **Steady State Filter**: Automatic filesystem observation ignores file modifications issued by Arbiter itself.
5. **Interference Guard**: Detects human presence and enforces a grace period to prevent collisions between the user and automation.
6. **Hardware Reset Guard**: Automatic hardware release ensures no keys are left in a stuck state if the engine process terminates unexpectedly.

## Getting Started and Installation

> [!NOTE]
> Arbiter uses a low-level Win32 API (WH_KEYBOARD_LL) to capture global hotkeys and spawn detached shell processes. Pre-compiled binaries in the zip file may be flagged by Windows Defender heuristics. For a frictionless experience, download the pre-compiled binaries via the powershell command below, compile locally using cargo install or add an explicit folder exclusion in Windows Security.

### Prerequisites

* Windows 10 or later
* Rust 1.70 or later for building from source

### Downloading Pre-built Binaries

1. Download the latest release from the [releases page](https://github.com/Sid-352/Project-Arbiter/releases/latest).
2. Extract the contents of the downloaded file.
3. Run the background service (as Administrator):
```bash
.\arbiter.exe
```

### Downloading via Powershell
Downloading directly can bypass some of the Windows Defender SmartScreen issues and reduces false-positive flags.
```powershell
Invoke-WebRequest -Uri "https://github.com/Sid-352/Project-Arbiter/releases/latest/download/arbiter-windows.zip" -OutFile "arbiter.zip"; Expand-Archive "arbiter.zip" -DestinationPath ".\arbiter"; Unblock-File -Path ".\arbiter\*.exe"
```

### Install via Cargo 
Compiling locally guarantees that the application won't run into any SmartScreen issues.
```bash
cargo install --git [https://github.com/Sid-352/Project-Arbiter.git](https://github.com/Sid-352/Project-Arbiter.git) arbiter-app arbiter-forge
```

### Building from Source

1. Clone the repository:
```bash
git clone https://github.com/Sid-352/Project-Arbiter.git
cd Project-Arbiter
```

2. Build both binaries:
```bash
cargo build --release --package arbiter-app
cargo build --release --package arbiter-forge
```

3. Run the background service (as Administrator):
```bash
.\target\release\arbiter.exe
```

### Quick Start (Windows)

1. Start `arbiter.exe` (Admin recommended).
2. Wait for the tray icon, then click `Open Forge` from the tray menu.
3. In Forge, create/save a decree and drop a matching file into your monitored folder to test.

Forge is expected to be launched by Arbiter App from the tray.

## Usage

### Running as a Background Service

```bash
cargo run --release --package arbiter-app
```

### Running the UI

Start the app first, then open Forge from the tray icon (`Open Forge`).

```bash
cargo run --release --package arbiter-forge
```

Direct Forge runs are only valid when Arbiter App is already running.

## License

MIT License

## Future Plans

- Conditional logic in the Decree sequence editor (branching steps based on analytical ward data).
- Signet vault encryption (AES-GCM key derivation is stubbed; full passphrase protection is pending because of startup issues, would love to hear some ideas on how to handle this).
