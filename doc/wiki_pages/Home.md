# Welcome to the Arbiter wiki

Arbiter is a **deterministic orchestrator** I made. It's designed to live in the background of a machine, watching for specific events and executing precise workflows with absolute reliability.

Most automation tools are certainly best-effort and jam packed with features, but Arbiter is built for stability. It cannot guess, drift, and is very light on the system.

### The Small Usage Mandate
I believe a background service shouldn't be a resource hog. Arbiter is built in Rust to stay incredibly lean (around 8MB at idle). It stays out of your way until it’s needed, then executes and returns to its steady state.

### How it Works
1. **The Vigil (Eyes)**: Watches your files, hotkeys, and processes for specific "Summons" triggers.
2. **The Atlas (Brain)**: A deterministic Finite State Machine (FSM) that drives your "Decree" sequences.
3. **The Signet (Guardrails)**: A hardened security layer that ensures Arbiter only touches what you've explicitly trusted.
4. **The Hand & Baton (Communicators)**: Hardware input and shell execution gates that carry out your actions.

### Why use Arbiter?
* **Zero Bloat**: Stays fast, stays small.
* **Hardened Security**: Path jailing and binary whitelisting are baked into the core.
* **Human-Aware**: Presence Abort logic detects when you're using your mouse or keyboard and yields control back to you instantly.
* **Deterministic**: No probabilistic logic. If a Decree is set, it runs exactly the same way, every time.

---
Check out the [[Lexicon]] to learn the language of the engine.
