# Context (Variable Mapping)

Context defines shared vocabulary of the system. Variables are injected into the execution context and can be referenced in any Decree step using `${env.variable_name}` syntax.

### Physical Attributes (Layer 1)
These variables are provided by filesystem without opening the file. They are always available for file-triggered events.

* `${env.file_name}`: Full filename with extension.
* `${env.file_path}`: Absolute path to the file.
* `${env.file_dir}`: Parent directory path.
* `${env.file_ext}`: File extension (lowercase).
* `${env.file_size_human}`: Formatted size (e.g. 1.2 MB).
* `${env.file_owner}`: Windows user or SID of the file owner.
* `${env.file_created_local}`: Creation timestamp in local time.

### Analytical Attributes (Layer 2)
These variables require triggering Ward to have Analytical depth enabled. Data is computed Just-in-Time only when a Decree step references the variable.

* `${env.content_sha256}`: SHA-256 hex digest of the file content.
* `${env.content_mime}`: MIME type detected from magic bytes.
* `${env.content_entropy}`: Shannon entropy (indicates compression or encryption).
* `${env.text_lines}`: Number of newlines in the file.

### Non-File Variables
When a Decree is triggered by hotkey or process, different variables are injected:

* `${env.hotkey_combo}`: Keyboard combination that fired the Summons.
* `${env.process_name}`: Name of the process that appeared.
* `${env.process_pid}`: Process ID of the detected program.
