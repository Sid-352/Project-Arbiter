# Arbiter: Metadata Extraction and Variable Mapping

This document specifies the data points the Vigil can extract from a file once a Summons is triggered. These variables are resolved into the execution context as `${env.variable_name}` for use inside Decree sequences.

## 1. Physical Attributes (Layer 1, always available)

Provided by the Windows filesystem (NTFS) without opening the file. Always computed for file-trigger events.

| Variable | Description | Example |
| --- | --- | --- |
| `${env.file_name}` | Full filename including extension. | `report.zip` |
| `${env.file_path}` | Absolute path to the file. | `C:\Downloads\report.zip` |
| `${env.file_dir}` | Parent directory path. | `C:\Downloads` |
| `${env.file_ext}` | File extension, lowercase. | `.zip` |
| `${env.file_size}` | Size in bytes. | `1048576` |
| `${env.file_size_human}` | Formatted size (KB, MB, GB). | `1.0 MB` |
| `${env.file_readonly}` | Whether the file is write-protected. | `false` |
| `${env.file_hidden}` | Whether the hidden attribute is set. | `false` |
| `${env.file_is_link}` | Whether this is a symlink or shortcut. | `false` |
| `${env.file_owner}` | Windows User/SID of the file owner. | `DESKTOP\Admin` |
| `${env.file_created_unix}` | Creation timestamp as Unix epoch. | `1712750400` |
| `${env.file_created_iso}` | Creation timestamp as ISO 8601 (UTC). | `2024-04-10T12:00:00Z` |
| `${env.file_created_local}` | Creation timestamp in local time. | `2024-04-10 17:30:00` |
| `${env.file_modified_iso}` | Last modification as ISO 8601 (UTC). | `2024-04-10T12:00:00Z` |
| `${env.file_modified_local}` | Last modification in local time. | `2024-04-10 17:30:00` |
| `${env.timestamp}` | Time the Summons was fired (UTC). | `2024-04-10T12:00:00Z` |
| `${env.timestamp_local}` | Time the Summons was fired (local). | `2024-04-10 17:30:00` |

## 2. Analytical Attributes (Layer 2, requires Analytical Ward)

> [!WARNING]
> These variables are only available if the triggering Ward has Layer 2 (Analytical) access enabled in the Conservatory. Requesting them on a Surface-only Ward will log a Signet Guard error and return null.

These are computed Just-in-Time: the file is only read if a Decree specifically references one of these keys.

| Variable | Description | Example |
| --- | --- | --- |
| `${env.content_sha256}` | SHA-256 hex digest of the file. | `a3f5...` |
| `${env.content_md5}` | MD5 hex digest of the file. | `d41d8c...` |
| `${env.content_mime}` | MIME type detected from magic bytes. | `application/zip` |
| `${env.content_entropy}` | Shannon entropy (0.0 to 8.0). High values indicate compression or encryption. | `7.9921` |
| `${env.text_lines}` | Newline count. Meaningful for text files only. | `1024` |

## 3. Image Attributes (Layer 2, placeholder)

> [!NOTE]
> These variables are defined in the data model but are not yet computed by the engine. They will return null until the image extraction pipeline is implemented.

| Variable | Description |
| --- | --- |
| `${env.img_dims}` | Pixel dimensions, e.g. `3840x2160`. |
| `${env.img_aspect}` | Aspect ratio as a float, e.g. `1.77`. |
| `${env.img_model}` | Camera or device model from EXIF data. |
| `${env.img_gps}` | Whether the image contains GPS coordinates. |

## 4. Non-File Trigger Variables

For Hotkey and Process Summons, file variables are not available. The following keys are injected instead.

| Variable | Source | Example |
| --- | --- | --- |
| `${env.hotkey_combo}` | Hotkey trigger | `ctrl+shift+a` |
| `${env.process_name}` | Process trigger | `notepad.exe` |
| `${env.process_pid}` | Process trigger | `4892` |

## 5. Variable Resolution Architecture

Arbiter evaluates variables using a two-stage resolution model:

1. **Eager (at trigger time):** All Layer 1 physical attributes are computed immediately when the Summons fires and cached in the execution context for the duration of the run.
2. **Lazy (on first access):** Layer 2 analytical attributes are computed only when a step in the Decree sequence attempts to interpolate that variable. The result is cached in `OnceLock` so it is computed at most once per sequence execution, even if referenced multiple times.