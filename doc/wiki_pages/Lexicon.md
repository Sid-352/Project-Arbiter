# Arbiter Lexicon (Dictionary)

To use Arbiter is to understand its core vocabulary. I use these terms to describe exactly what the engine is doing and why, and many of them are certainly easier to understand than the actual technical terms.

### 1. Vigil
**Eyes.** Pluggable observers that listen for events. Whether it's a file appearing in a folder, a global hotkey, or a specific program starting, the Vigil is what detects the initial spark of a workflow.

### 2. Atlas
**Brain.** A Finite State Machine that manages the overall flow of the engine. It keeps track of whether Arbiter is **Idle**, **Executing**, **Yielded** (paused for human input), or in a **Faulted** state.

### 3. Signet
**Guardrails.** Arbiter's security vault. It manages **Trusted Roots** (folders Arbiter is allowed to touch) and **Baton Whitelists** (allowed shell commands). If an action isn't signed by the Signet, it doesn't happen.

### 4. Summons
**Trigger.** A specific condition defined in the Vigil. A Summons is the "If this happens..." part of your workflow.

### 5. Decree
**Sequence.** A set of ordered actions triggered by a Summons. A Decree is the "...then do this" part of your workflow.

### 6. Wards & Conservatory
**Watch-Zones.** A **Ward** is a folder the Vigil is actively monitoring. The **Conservatory** is the collection of all your active Wards.

### 7. Presence Abort
**Human Input Detection.** Arbiter is designed to be polite. If it's running a sequence and detects you moving your mouse or typing, it will instantly stop in its tracks, pausing or yielding control to the user.

### 8. Baton
**Shell Execution.** When Arbiter needs to run an external program (like 7-Zip or FFmpeg), it uses the Baton. All Baton commands must be explicitly whitelisted in the Signet.

### 9. Inscribe
**File Operations.** The logic for moving, copying, or renaming files securely across your machine.
