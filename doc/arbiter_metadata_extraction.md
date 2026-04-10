# Arbiter: Metadata Extraction & Variable Mapping

This document specifies the data points the **Vigil** can extract from a file once a **Summons** is triggered. These attributes are injected into the **Governor's** context as `${env.variable_name}` for use in **Ordinances**.

## 1. Physical Attributes (The OS Layer)

These are "free" data points provided by the Windows filesystem (NTFS) without needing to read the actual content of the file.

| Variable | Description | Example Value | 
| ----- | ----- | ----- | 
| `${env.file_size}` | Total size in bytes. | `1048576` | 
| `${env.file_size_human}` | Formatted size (KB, MB, GB). | `1.0 MB` | 
| `${env.file_ext}` | File extension (lowercase). | `.png` | 
| `${env.file_readonly}` | Boolean: Is the file write-protected? | `true` | 
| `${env.file_hidden}` | Boolean: Is the hidden attribute set? | `false` | 
| `${env.file_created_unix}` | Creation timestamp (Unix epoch). | `1712750400` | 
| `${env.file_modified_iso}` | Last modification (ISO 8601). | `2024-04-10T12:00:00Z` | 
| `${env.file_owner}` | Windows User/SID of the file owner. | `DESKTOP-ARBITER\Admin` | 
| `${env.file_is_link}` | Boolean: Is this a shortcut or symlink? | `false` | 

## 2. Integrity & Signature (The Content Layer)

For higher-security Ordinances, Arbiter can perform a "Quick Peek" at the file content.

* **Magic Bytes (MIME Type):** Identification by file header rather than extension.
  * *Variable:* `${env.content_mime}` (e.g., `application/x-zip-compressed`).

* **Checksums (Hashes):**
  * *Variable:* `${env.content_sha256}`.
  * *Variable:* `${env.content_md5}`.

* **Entropy Score:** Measures the randomness of data. High entropy (~0.8+) usually indicates encryption or compressed archives.
  * *Variable:* `${env.content_entropy}`.

## 3. The Origin Layer (Windows Specifics) - *needs to be tested*

This is a "Secret Weapon" for automation. Windows stores metadata in **Alternative Data Streams (ADS)** that most users never see.

* **Zone.Identifier:** When you download a file, Windows tags it with the "Zone" it came from.
  * *Variable:* `${env.origin_zone}` (0: Local, 3: Internet, 4: Restricted).
  * *Variable:* `${env.origin_url}`: The specific URL the file was downloaded from (if saved by the browser).
  * *Variable:* `${env.origin_host}`: The domain (e.g., `github.com`).

* **Parent Process:**
  * *Variable:* `${env.origin_process_name}`: The name of the app that created the file (e.g., `chrome.exe` or `powershell.exe`).

## 4. Specialist Metadata (The Deep Vigil)

These extractors are only triggered if the Ordinance requires specific deep-data points.

### 4.1 Imagery & Vision
* `${env.img_dims}`: (e.g., `3840x2160`).
* `${env.img_aspect}`: Ratio (e.g., `1.77`).
* `${env.img_model}`: Camera/Device model from EXIF.
* `${env.img_gps}`: Boolean: Does the image contain location data?

### 4.2 Text & Documentations
If the file is recognized as a text document (via MIME or extension):
* `${env.text_lines}`: Total line count.

## 5. Layered Rule Logic (Conditional Grouping)

This allows for "Tiered Verification" using these variables. You can layer logic to create hyper-specific filters.

### Example: The "Safe Download" Ordinance
**Summons:** File created in `Downloads/`.
**Layer 1 (Physical):**
* IF `${env.file_ext}` == `.exe`

**Layer 2 (Integrity):**
* AND IF `${env.bin_publisher}` == `Unknown`

**Action:**
* **InscribeMove** to `Quarantine/`.
* **Shell** `notify-send "Unsigned executable from unknown source blocked."`

## 6. Implementation Architecture

To maintain the "Discrete Servant" philosophy, Arbiter uses a **Just-In-Time (JIT) Extraction** model:

1. **Level 0:** Always triggered. Pulls Section 1 (Physical).
2. **Level 1:** Only triggered if a conditional node in the **Forge** (Node Graph) asks for a specific variable from Sections 2 or 4.

This ensures that the engine doesn't waste CPU cycles hashing a 50GB file unless you've specifically told it: *"Only move this if the SHA256 matches my database."*