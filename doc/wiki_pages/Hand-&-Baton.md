# Hand & Baton

Hand and Baton are the execution bridges of the engine. These components carry out physical and system-level actions defined in Decree sequences.

### Hand (Hardware Input)
Hand provides the ability to simulate human input. It is used for actions that require interaction with OS UI elements that do not have accessible APIs.

* **Keypress**: Emits single keys or combinations (e.g. `Win+D`, `Alt+F4`).
* **Type**: Emits a string of characters as if typed by the user.

### Baton (Shell Bridge)
Baton is the bridge to the system shell. It allows Arbiter to launch external programs to perform tasks like compression, media encoding, or script execution.

* **Security**: Baton will only execute binaries that are explicitly whitelisted in Signet.
* **Detached Execution**: By default, Baton launches processes in a detached state. This ensures that even if a launched program hangs, Atlas and the rest of the engine remain responsive.
* **Non-Interactive**: Baton is designed for automated tasks and does not capture or provide interactive STDIN/STDOUT during the execution of a Decree.

### Presence Abort Integration
Both Hand and Baton are subject to Presence detection. If the user moves the mouse or presses a key while Hand is attempting to simulate input, the engine instantly yields control to the user to prevent hardware contention.
