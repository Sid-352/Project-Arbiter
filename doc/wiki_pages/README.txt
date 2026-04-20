Project Arbiter

Arbiter is a Windows automation engine with a tray-hosted runtime (arbiter.exe) and a UI editor (arbiter-forge.exe).

Quick Start (Windows)

1) Start arbiter.exe (Run as Administrator recommended).
2) Wait for the tray icon, then click Open Forge.
3) Create or edit a decree in Forge and click COMMIT TO ENGINE.
4) Trigger it (file event, hotkey, process, or simulate/manual run).

Components

- arbiter-core: rules, state, triggers, safety checks.
- arbiter-bridge: execution layer (input, file ops, shell).
- arbiter-app: tray host + runtime lifecycle.
- arbiter-forge: decree editor and telemetry UI.

Build and Run

Build release binaries:
  cargo build --release --package arbiter-app
  cargo build --release --package arbiter-forge

Run app (host runtime):
  cargo run --release --package arbiter-app

Run Forge directly (only if app is already running):
  cargo run --release --package arbiter-forge

Configuration Files

Arbiter creates arbiter-data next to the executables on first run.

- arbiter-data\ledger.json: decrees and wards.
- arbiter-data\signet.vault: trusted paths and execution allowlist.
- arbiter-data\logs\: runtime logs.

Safety Model (Short)

- File operations are restricted to Signet trusted roots.
- Shell executions are restricted by baton allowlist.
- Presence guard can yield active runs on user input.
- Runtime includes recursion and debounce protections.

Notes

- Forge is app-owned; standalone Forge launch is intentionally blocked.
- Use tray actions (Pause Engine, Reset Engine, Open Forge) for control.
