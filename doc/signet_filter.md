# Arbiter: The Signet Filter Specification

The **Signet Filter** is the authoritative security layer. It operates on a **User-Defined Trust Model**. Arbiter never assumes the "nature" of a folder; it only knows what permissions you have granted to a specific path (Ward).

## 1. The Identity Layer (Recursion Guard)
* **Mechanism:** Every action performed by the engine is registered in an **Active Operation Table**.
* **Logic:** If an incoming system signal (File Created/Modified) matches a fingerprint in the table, it is discarded. 
* **Result:** Arbiter remains "blind" to its own actions to prevent infinite loops.

## 2. The Authority Layer (Ward Scoping)
Instead of arbitrary "Hot Zones," you define the **Extraction Depth** for every folder added to the Conservatory.

### Layer 1: Surface Access (Standard)
* **Scope:** Applied to all Wards by default.
* **Permissions:** Allows the Vigil to read NTFS/OS-level metadata only.
* **Available Data:** Name, Path, Size, Created/Modified Timestamps, Attributes.
* **Performance:** Instantaneous. No file handles are opened.

### Layer 2: Deep Access (Analytical)
* **Scope:** Manually enabled by the user for specific folders (e.g., your "Edge" folder or "Z:\HDD").
* **Permissions:** Grants the Vigil "Read" access to the file's binary content and Alternative Data Streams.
* **Available Data:** MIME/Magic Bytes, Hashes (SHA256), Origin (Zone.Identifier), Media Metadata, Code Analysis.
* **Reliability:** If a piece of metadata is missing (e.g., a "dumb" downloader didn't write a URL), the variable returns `null`.
* **Performance:** JIT (Just-In-Time). The file is only read if an **Ordinance** specifically requests a Deep Variable.

## 3. The Execution Logic (Baton & Presence)
The final gate before the engine physically moves or executes code.
* **The Baton:** External scripts (Python/EXE) require an explicit one-time user grant to run within a specific Ward.
* **Somatic Abort:** If an Ordinance requires **The Hand** (Mouse/Keyboard), the Signet checks the **Presence** buffer. If you are "In-Seat" (moving the mouse), the order is refused.

## 4. Summary of the 2-Layer Workflow
1.  **Signal Received:** Is this a file event I caused? -> Yes: **Discard**.
2.  **Ward Verification:** Is this path in my Conservatory? -> No: **Discard**.
3.  **Permission Check:** * Does the Ordinance need "Deep Data" (like a Hash)? 
    * If yes, does this Ward have **Layer 2** enabled? -> No: **Log Error/Abort**.
4.  **Baton Check:** Do I have permission to run the required script? -> No: **Prompt User**.
5.  **Presence Check:** Is the user currently using the peripherals? -> Yes: **Yield/Pause**.

---