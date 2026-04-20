# Atlas (Execution & FSM)

Atlas acts as brain of the engine. Atlas drives the execution of Decrees using a deterministic Finite State Machine.

### The State Machine
Atlas ensures the engine is always in a known and debuggable state:
* **Idle**: Waiting for a Summons from Vigil.
* **Executing**: Actively running a Decree sequence.
* **Yielded**: Paused because human activity was detected.
* **Faulted**: Stopped because an error or security violation occurred.

### Deterministic Flow
Atlas processes one Decree at a time. This is mainly to prevent race conditions and ensures that hardware resources (like the mouse or keyboard) are never fought over by competing sequences.

### Presence Yield
When Presence detection reports human activity, Atlas transitions the engine to a Yielded state, which basically allows the user to maintain total control of the machine. The engine only resumes or completes once the system is clear of human interference.
