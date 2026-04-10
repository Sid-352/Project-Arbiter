# GEMINI.md - Project Arbiter

## Project Overview
**Arbiter** is an industrial-grade, deterministic system orchestration and automation engine. It is designed as a silent background service that executes complex physical and system-level workflows based on explicitly defined orchestration graphs (Nodes). Unlike probabilistic models, Arbiter prioritises absolute security, structural stability, and deterministic execution.

The project is a Rust workspace consisting of four primary crates:
- **`arbiter-core`**: The "Brain". Manages logic, state (Atlas FSM), signals (Vigil), and security permissions (Signet).
- **`arbiter-bridge`**: The "Muscle". Handles low-level execution including hardware input (Hand), file system operations (Inscribe), and sub-process management (Baton).
- **`arbiter-app`**: The "Host". A thin wrapper providing a background tray application, Tokio runtime initialization, and structured logging.
- **`arbiter-forge`**: A visual terminal interface (egui-based) for monitoring logs and engine state.

## Architecture (The Arbiter Lexicon)
- **The Atlas (`arbiter-core/atlas`)**: The Finite State Machine (FSM) orchestrator that maps triggers to sequences.
- **The Vigil (`arbiter-core/vigil`)**: Pluggable observation listeners for hotkeys, file monitoring, and system events.
- **The Presence (`arbiter-core/presence`)**: Human-input detection that triggers a "Somatic Abort" to yield control to the user.
- **The Signet (`arbiter-core/signet`)**: A security vault managing "Trusted Roots" and execution permissions.
- **The Hand (`arbiter-bridge/hand`)**: Hardware bridge for mouse and keyboard routing via `enigo`.
- **The Inscribe (`arbiter-bridge/inscribe`)**: Secure file system IO manager.
- **The Baton (`arbiter-bridge/shell`)**: Hardened sub-process utility with whitelist guards.

## Building and Running

### Prerequisites
- **Rust Toolchain**: 2021 Edition.
- **Target OS**: Primarily Windows (as indicated by `windows_subsystem` and tray logic), but core logic is platform-agnostic where possible.

### Key Commands
- **Build All**: `cargo build`
- **Run Background Service**: `cargo run -p arbiter-app` (Note: In release, this runs as a background process without a console).
- **Run Terminal UI**: `cargo run -p arbiter-forge`
- **Run Tests**: `cargo test`
- **Release Build**: `cargo build --release` (Includes aggressive size optimisations like `opt-level = "z"`, `lto`, and `strip`).

## Development Conventions

### 1. Data Contracts (`ordinance.rs`)
All shared vocabulary (Triggers, Actions, Nodes) must be defined in `arbiter-core/src/ordinance.rs`. This ensures a single source of truth for the Atlas, Bridge, and Terminal.

### 2. Structured Logging
Arbiter uses `tracing` for all logging.
- **Stdout**: Real-time compact logs during development.
- **File**: Persistent logs stored in `doc/logs/arbiter.log`, designed to be tailed by `arbiter-forge`.

### 3. Security Guards
Every new execution feature must integrate with the **Signet Filter**:
- **Conservatory Guard**: Disk operations must be clamped to "Trusted Roots".
- **Baton Guard**: Shell executions must be whitelisted.
- **Steady State Filter**: Prevents infinite loops by ignoring file events triggered by Arbiter itself.

### 4. Feature Flags
`arbiter-core` is highly modular. Use features to toggle dependencies:
- `vigil-fs`: File system watching (`notify`).
- `vigil-keys`: Global hotkeys (`global-hotkey`).
- `presence`: Human input detection (`rdev`).
- `signet`: Encrypted configuration (`aes-gcm`).

## Key Files & Directories
- `arbiter-core/src/ordinance.rs`: Core data structures and types.
- `arbiter-core/src/atlas.rs`: The main FSM loop logic.
- `arbiter-bridge/src/executor.rs`: The central execution gate for the bridge.
- `arbiter-app/src/main.rs`: Entry point and logging initialisation.
- `doc/`: Contains system logs and detailed specifications for the Signet filter and Arbiter metadata extraction.
