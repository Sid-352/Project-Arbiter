# Project Arbiter

Arbiter is an industrial-grade, deterministic system orchestration and automation engine. It acts as a silent background service designed to perform complex physical and system-level workflows reliably. It prioritizes absolute security, structural stability, and protection against unbounded behavior.

## Core Philosophy

* **Deterministic Execution**: Actions follow rigid and explicitly defined orchestration graphs instead of probabilistic models. Execution paths are strictly predictable and bounded.
* **Headless Default**: Arbiter operates primarily as a silent background tray application. File system hooks, hotkey triggers, and hardware queues function independently of a visual interface.

## Architecture

Arbiter is split into four strictly walled component crates to isolate scope and enforce native restrictions.

### 1. arbiter-core (The Brain)
Handles all logical state, permissions, configurations, and signal observation. It provides data contracts but executes no OS mutations.
* **The Vigil**: Pluggable observation listeners for hotkeys and file monitoring.
* **The Atlas**: The Finite State Machine evaluation loop that maps triggers to sequences.
* **The Signet**: Secure configuration vault managing trusted paths and command whitelists.
* **The Filter**: In-memory path lock state that prevents infinite event observation loops.

### 2. arbiter-bridge (The Muscle)
A single-responsibility hardware and file execution layer. It processes incoming logical directives through a global queuing lock.
* **The Runner**: Background orchestration task that manages sequential action execution.
* **The Hand**: Physical keyboard and mouse routing handler with coordinate bounds checks.
* **The Inscribe**: Secure file system IO manager handling localized file manipulation using PathBuf for cross-platform safety.
* **The Baton**: Hardened sub-process launching utility handling independent executions.

### 3. arbiter-app (The Host)
The thin entrypoint wrapper managing lifecycle state, custom daily rolling loggers, Tokio asynchronous runtime initialization, and system-tray integration.

### 4. arbiter-forge (The Terminal)
A Slint-based visual interface for monitoring live telemetry and managing engine state. It connects to the host via high-performance Named Pipe IPC.

## Safety and Fallbacks (The Guards)

Arbiter is mechanically prevented from operating beyond user-defined constraints. Six critical systems guarantee runtime safety.

1. **The Conservatory Guard**: All disk operations are clamped to a user-defined whitelist of Trusted Roots.
2. **The Baton Guard**: Arbitrary shell and process executions are strictly bounded by a pre-calculated whitelist.
3. **The Hardware Guard**: Coordinate constraints enforce bounding pointer logic within known monitor dimensions.
4. **The Steady State Filter**: Automatic filesystem observation ignores file modifications issued by Arbiter itself.
5. **The Somatic Lock**: Detects human presence and enforces a grace period to prevent collisions between the user and automation.
6. **The Panic to Safe Guard**: Automatic hardware release ensures no keys are left in a stuck state if the engine process terminates unexpectedly.

## Advanced Features

* **Real-time Telemetry**: Sub-millisecond log streaming from the background service to the UI using Windows Named Pipes.
* **Scope-bound Presence Sensitivity**: Granular control over human input detection, allowing specific sequences to ignore mouse movement while remaining reactive to keyboard safety yields.
* **Custom Daily Rolling Logs**: Automated log management using a custom writer that organizes history by date in the arbiter-data directory.
