# Project Vassal

Vassal is an industrial-grade, deterministic system orchestration and automation engine. It acts as a silent background service designed to perform complex physical and system-level workflows reliably, prioritizing absolute security, structural stability, and an uncompromising guard against unbounded behavior.

## Core Philosophy

* **Deterministic Execution**: Actions follow rigid, explicitly defined orchestration graphs (Nodes) rather than probabilistic models. Execution paths are strictly predictable and bounded.
* **Headless Default**: Vassal operates primarily as a silent background tray application. File system hooks, hotkey triggers, and hardware queues function completely independently of a visual interface.

## Architecture

Vassal is split into three strictly walled component crates to isolate scope and enforce native restrictions:

### 1. vassal-core (The Brain)
Handles all logical state, permissions, configs, and signal observation. Provides data contracts but executes zero OS mutations.
* **The Vigil**: Pluggable observation listeners (hotkeys, file monitoring).
* **The Atlas**: The Finite State Machine (FSM) evaluation loop that maps triggers to sequences.
* **The Filter**: In-memory path lock state that prevents infinite event observation loops.

### 2. vassal-bridge (The Muscle)
A single-responsibility hardware and file execution layer. Processes incoming logical directives via a global queuing lock.
* **The Hand**: Physical keyboard and mouse routing handler with coordinate bounds checks.
* **The Inscribe**: Secure file system IO manager handling localized file manipulation.
* **The Baton**: Hardened sub-process launching utility handling independent executions.

### 3. vassal-app (The Host)
The thin entrypoint wrapper managing lifecycle state, structured tracing loggers, Tokio asynchronous runtime initialization, and system-tray integration without bloating the domain logic.

## Safety and Fallbacks (The Guards)

Vassal is mechanically prevented from operating beyond user-defined constraints. Four critical systems guarantee runtime safety:

1. **The Conservatory Guard**: All disk operations (move, copy, delete) are clamped to a user-defined whitelist of "Trusted Roots". A compromised configuration attempting to manipulate isolated system files will instantly throw an error and reject the execution.
2. **The Baton Guard**: Arbitrary shell and process executions are strictly bounded. Every sub-process target is mapped against a pre-calculated whitelist. Shell operations invoke direct processes rather than arbitrary interpreters, bypassing malicious heuristic threat flags securely.
3. **The Hardware Guard**: Coordinate constraints enforce bounding pointer logic strictly within known monitor dimensions before passing instructions to underlying OS native calls.
4. **The Steady State Filter**: Automatic filesystem observation ignores file modifications issued by Vassal itself, guaranteeing that infinite modification loops are mathematically impossible.

## Dynamic Pipeline

* **Event Extraction**: Hooks (hotkeys or file drops) map incoming localized contexts into a transient environment context.
* **Deep Interpolation**: Engine Nodes securely interpolate dynamic keys (like file paths or names) deep into final Execution configurations using token replacement (e.g. `${env.file_path}`).
* **Execution Queue**: Handled through a singleton lock assuring synchronous linear execution, cleanly avoiding destructive OS racing states.
