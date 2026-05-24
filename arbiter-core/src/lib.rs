pub mod atlas;
pub mod decree;
pub mod ledger;
pub mod protocol;

#[cfg(any(
    feature = "vigil-sys",
    feature = "vigil-fs",
    feature = "vigil-keys",
    feature = "vigil-clipboard"
))]
pub mod vigil;

#[cfg(feature = "presence")]
pub mod presence;

#[cfg(feature = "signet")]
pub mod signet;

pub mod filter;

pub fn normalize_windows_path(path: &str) -> String {
    fn is_drive_root(p: &str) -> bool {
        let b = p.as_bytes();
        b.len() == 3 && b[1] == b':' && b[2] == b'\\'
    }
    let mut out = path.trim().replace('/', "\\");

    while out.ends_with('\\') && !is_drive_root(&out) {
        out.pop();
    }
    out
}

#[cfg(test)]
mod test {
    use super::normalize_windows_path;

    #[test]
    fn removes_trailing_slash() {
        assert_eq!(
            normalize_windows_path(r"C\:temp\folder\"),
            r"C\:temp\folder"
        );
    }

    #[test]
    fn preserve_drive_root() {
        assert_eq!(normalize_windows_path(r"C:\"), r"C:\");
    }
    #[test]
    fn normalizes_forward_slashes() {
        assert_eq!(normalize_windows_path("C:/temp/test/  "), r"C:\temp\test");
    }
    #[test]
    fn trim_whitespace() {
        assert_eq!(normalize_windows_path("  C:/temp/test/  "), r"C:\temp\test");
    }
    #[test]
    fn preserve_drive_root_after_normalization() {
        assert_eq!(normalize_windows_path("C:/"), r"C:\");
    }
}
