# Vigil (Observation & Triggers)

Vigil acts as eyes of the engine. Vigil monitors the environment for specific signals that trigger a Summons.

### Summons Triggers
Vigil currently supports three primary trigger types:
* **File System**: Monitors a Ward for new or modified files.
* **Hotkey**: Listens for global keyboard combinations.
* **Process**: Detects when a specific executable starts running on the system.

### Wards & Conservatory
Ward is a specific directory path that Vigil is assigned to watch. Conservatory is the collection of all active Wards. Every Ward has a defined security depth.

### Extraction Layers
* **Layer 1 (Surface)**: Provides basic NTFS metadata like name, size, and timestamps. This is always active and has zero performance impact.
* **Layer 2 (Analytical)**: Allows Vigil to read file content for deep data like SHA256 hashes or MIME types. This layer must be explicitly enabled for a Ward.
