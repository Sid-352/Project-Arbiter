# Pattern Matching 

Guide and help for defining patterns to scan for in the forge. The system uses a slightly separate matching logic for monitoring folders and filtering file events. These patterns use the **Glob Syntax** (yes that's a real thing).

### Wildcard Syntax

| Pattern | Description | Example |
| :--- | :--- | :--- |
| `*` | Matches any number of characters within a single directory level. | `*.txt` matches `file.txt` but not `sub/file.txt`. |
| `**` | Matches any number of characters across multiple directory levels. | `src/**/*.rs` matches all Rust files in `src` and any subfolders. |
| `?` | Matches exactly one character. | `img_??.jpg` matches `img_01.jpg` but not `img_1.jpg`. |
| `[abc]` | Matches any single character contained within the brackets. | `data_[01].csv` matches `data_0.csv` or `data_1.csv`. |
| `[a-z]` | Matches a range of characters. Ranges can be combined (e.g., `[a-zA-Z0-9]`). *Note: On Windows, `[a-z]` is effectively the same as `[a-zA-Z]` due to case-insensitivity.* | `log_[0-9].txt` matches `log_5.txt`. `[a-z0-9]*` matches any alphanumeric name. |
| `[!abc]` | Negation: matches any character **not** in the brackets. | `test_[!0-9].rs` matches `test_a.rs` but not `test_1.rs`. |
| `{a,b}` | Alternation: matches any of the comma-separated patterns. | `*.{jpg,png,gif}` matches all three image types. |
| `\` | Escapes a special character. | `file\*.txt` matches the literal name `file*.txt`. |

### Scope of Matching

Arbiter applies these patterns differently depending on the context:

1.  **Wards:**
    Patterns defined in a Ward (e.g., in a `FileCreated` Summons) are matched against the **filename only**. 
    *Example:* A Ward watching `C:\Downloads` with pattern `*.zip` will trigger when `manual.zip` is created.

2.  **Signet:**
    Patterns used in the Signet "Jail" or "Conservatory" are matched against the **full absolute path**.
    *Example:* A restriction `**\Temp\**` will block any file operation involving a folder named "Temp" anywhere in the path.

### Glob and Regular Expressions

If you are used to standard Regular Expressions, keep these differences in mind:

*   **Anchoring:** Globs are **implicitly anchored**. You do not need `^` or `$`. The pattern `*.txt` automatically behaves like the regex `^.*\.txt$`.
*   **The `+` Symbol:** In Glob syntax, `+` is just a literal character. To match "one or more" characters, use `?*` (exactly one character followed by zero or more).
*   **The `.` Symbol:** In Regex, `.` matches any character. In Glob, `.` is a literal dot. Use `?` for a single-character wildcard.

### Processing Rules

**Case Sensitivity:**
On Windows systems, Arbiter processes all patterns as **case-insensitive** by default to align with the operating system's behavior. `*.JPG` and `*.jpg` are treated as identical.

**Performance:**
Patterns are pre-compiled into a finite state machine (via the `globset` engine) at the moment a Decree is loaded or a Ward is spawned. This ensures that even complex patterns with recursive wildcards (`**`) can filter thousands of file system events with minimal CPU latency.

### Practical Examples

*   **Capture all documents:** `*.{doc,docx,pdf,txt}`
*   **Numbered sequences:** `frame_[0-9][0-9][0-9].png` (matches `frame_001.png` through `frame_999.png`)
*   **Ignore hidden files (Unix style):** `[!.]*`
*   **Recursive source monitoring:** `project_root/**/*.c`
