# Signet (Security & Permissions)

Signet acts as basically the guardrails of the engine. Signet establishes hard boundaries for what Arbiter is permitted to do on the system.

### Trusted Roots
Arbiter operates under a "Default Deny" posture. Inscribe operations (moving or copying files) are only permitted if the destination is within a directory listed as a Trusted Root.

### Baton Whitelist
Shell execution is strictly controlled. Baton will only launch binaries that are explicitly listed in the whitelist. If a Decree attempts to run an unlisted program, Signet blocks the execution and logs a security violation.

### Steady State Filter
To prevent infinite loops, Signet maintains an identity table of all file operations performed by the engine. If Vigil detects a file event caused by Arbiter itself, Signet discards the event. This ensures the system remains in a steady state.

### Presence Abort
Signet works with Presence detection to ensure the engine does not interfere with the user. If mouse or keyboard activity is detected while a Decree is running, the engine can yield or abort based on configuration.
