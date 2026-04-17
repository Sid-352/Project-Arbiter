# Arbiter: Signet Filter Specification

The **Signet Filter** is the authoritative security layer. It operates on a **User-Defined Trust Model**. Arbiter never assumes the nature of a folder; it only knows what permissions you have explicitly granted to a specific watched path (Ward).

## 1. Identity Layer

- **Mechanism:** Every action performed by the engine is registered in an active operation table.
- **Logic:** If an incoming filesystem event matches a fingerprint already in the table, it is discarded.
- **Result:** Arbiter remains blind to its own file mutations which prevents infinite trigger loops.

## 2. Authority Layer

The user defines the **Extraction Depth** for every folder added to the Conservatory. There is no implicit access.

### Layer 1: Surface Access (Default)

- Applied to all Wards by default.
- Allows the Vigil to read NTFS/OS-level metadata only.
- Available data: Name, Path, Size, Created/Modified timestamps, Attributes.
- Performance: Instantaneous. No file handles are opened.

### Layer 2: Analytical Access

- Manually enabled by the user for specific folders.
- Grants the Vigil read access to the file's binary content.
- Available data: MIME/Magic Bytes, SHA256 hash, MD5 hash, Shannon Entropy, line count.
- Reliability: If a variable cannot be computed (e.g., binary file for a text-lines query), the variable resolves to `null`.
- Performance: Just-in-Time. The file is only read if a Decree specifically requests an analytical variable.

## 3. Execution Gate

The final gate before the engine physically moves files or executes code.

- **Baton**: External shell commands require an explicit user-defined whitelist entry. Unlisted binaries are blocked at the execution boundary.
- **Interference Guard**: If a Decree drives keyboard or mouse input, the engine checks the Presence buffer. If the user is detected as active (moving the mouse or pressing keys), the sequence yields or aborts depending on the per-Decree configuration.

## 4. Workflow Summary

The following checks occur in order for every incoming signal:

1. **Identity check**: Did Arbiter cause this event? Yes: Discard.
2. **Ward verification**: Is this path in a configured Ward? No: Discard.
3. **Layer check**: Does the Decree need an analytical variable? If yes, does this Ward have Layer 2 enabled? No: Log error and abort.
4. **Baton check**: Does the Decree execute a shell command? Is the binary whitelisted? No: Abort.
5. **Interference check**: Is the user currently active on the machine? Yes: Yield or abort based on Decree configuration.