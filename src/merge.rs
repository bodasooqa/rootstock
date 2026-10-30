//! Binary detection, the three-way merge contract and conflict-marker detection.

/// Number of leading bytes scanned for NUL when classifying a file as binary.
const BINARY_SCAN_LEN: usize = 8000;

/// Result of a three-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    Clean(Vec<u8>),
    Conflict(Vec<u8>),
}

/// Three-way merge of text; callers must check `is_binary` first.
///
/// `path` only names the file in error messages.
/// Conflicts are written as `<<<<<<< project`, `=======`, `>>>>>>> <target_label>`.
pub trait Merge3 {
    fn merge3(
        &self,
        path: &str,
        base: &[u8],
        project: &[u8],
        target: &[u8],
        target_label: &str,
    ) -> Merged;
}

/// True when the bytes contain NUL in the first 8000 bytes or are not valid UTF-8.
pub fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(BINARY_SCAN_LEN).any(|&b| b == 0) || std::str::from_utf8(bytes).is_err()
}

/// True when a `<<<<<<< ` line, a `=======` line and a `>>>>>>> ` line occur in order.
///
/// Mirrors `.harness/update/conflict-markers.sh`; binary input gives false.
pub fn has_markers(bytes: &[u8]) -> bool {
    if is_binary(bytes) {
        return false;
    }
    let mut state = 0;
    for line in bytes.split(|&b| b == b'\n') {
        if line.starts_with(b"<<<<<<< ") {
            state = 1;
        } else if state == 1 && (line == b"=======" || line == b"=======\r") {
            state = 2;
        } else if state == 2 && line.starts_with(b">>>>>>> ") {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn has_markers_needs_all_three_in_order() {
        assert!(!has_markers(b"title\n=======\nbody\n"));
        assert!(!has_markers(b"<<<<<<< a\nx\n"));
        assert!(!has_markers(b">>>>>>> t\n=======\n<<<<<<< p\n"));
        assert!(has_markers(b"<<<<<<< p\n=======\n>>>>>>> t\n"));
        assert!(has_markers(b"<<<<<<< p\r\n=======\r\n>>>>>>> t\r\n"));
        assert!(!has_markers(b"<<<<<<< p\n=======\n>>>>>>> t\n\0"));
        let conflict = b"a\n<<<<<<< project\nmine\n=======\ntheirs\n>>>>>>> template v1.1.0\nc\n";
        assert!(has_markers(conflict));
    }

    #[test]
    fn binary_detection() {
        assert!(is_binary(b"ab\0cd"));
        assert!(is_binary(&[0xff, 0xfe, b'a']));
        assert!(!is_binary("héllo\n".as_bytes()));
        let mut late = vec![b'a'; BINARY_SCAN_LEN];
        late.push(0);
        assert!(!is_binary(&late));
    }
}
