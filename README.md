# Project Arbiter

Arbiter is an industrial-grade, deterministic system orchestration and automation engine. It acts as a silent background service designed to perform complex physical and system-level workflows reliably. It prioritizes absolute security, structural stability, and protection against unbounded behavior.

## Core Philosophy

* **Deterministic Execution**: Actions follow rigid and explicitly defined orchestration graphs instead of probabilistic models. Execution paths are strictly predictable and bounded.
* **Headless Default**: Arbiter operates primarily as a silent background tray application. File system hooks, hotkey triggers, and hardware queues function independently of a visual interface.

## Architecture

Arbiter is split into three strictly walled component crates to isolate scope and enforce native restrictions.

### 1. arbiter-core (The Brain)
Handles all logical state, permissions, configurations, and signal observation. It provides data contracts but executes no OS mutations.
* **The Vigil**: Pluggable observation listeners for hotkeys and file monitoring.
* **The Atlas**: The Finite State Machine evaluation loop that maps triggers to sequences.
* **The Filter**: In-memory path lock state that prevents infinite event observation loops.

### 2. arbiter-bridge (The Muscle)
A single-responsibility hardware and file execution layer. It processes incoming logical directives through a global queuing lock.
* **The Hand**: Physical keyboard and mouse routing handler with coordinate bounds checks.
* **The Inscribe**: Secure file system IO manager handling localized file manipulation.
* **The Baton**: Hardened sub-process launching utility handling independent executions.

### 3. arbiter-app (The Host)
The thin entrypoint wrapper managing lifecycle state, structured tracing loggers, Tokio asynchronous runtime initialization, and system-tray integration.

## Safety and Fallbacks (The Guards)

Arbiter is mechanically prevented from operating beyond user-defined constraints. Four critical systems guarantee runtime safety.

1. **The Conservatory Guard**: All disk operations are clamped to a user-defined whitelist of Trusted Roots. A compromised configuration attempting to manipulate isolated system files will throw an error and reject the execution.
2. **The Baton Guard**: Arbitrary shell and process executions are strictly bounded. Every sub-process target is mapped against a pre-calculated whitelist. Shell operations invoke direct processes rather than arbitrary interpreters.
3. **The Hardware Guard**: Coordinate constraints enforce bounding pointer logic within known monitor dimensions before passing instructions to underlying OS native calls.
4. **The Steady State Filter**: Automatic filesystem observation ignores file modifications issued by Arbiter itself to ensure that infinite modification loops are impossible.

## Dynamic Pipeline

* **Event Extraction**: Hooks map incoming localized contexts into a transient environment context.
* **Deep Interpolation**: Engine Nodes securely interpolate dynamic keys into final execution configurations using token replacement like `${env.file_path}`.
* **Execution Queue**: Handled through a singleton lock ensuring synchronous linear execution and avoiding destructive OS racing states.
