//! Binary detection and three-way text merge with conflict markers.

use diffy::{ConflictStyle, IncompleteHunkStyle, MergeOptions};

/// Number of leading bytes scanned for NUL when classifying a file as binary.
const BINARY_SCAN_LEN: usize = 8000;
/// Conflict marker length tried first; grown until no input line can be mistaken for a marker.
const MIN_MARKER_LEN: usize = 7;
const MARKER_CHARS: &[u8] = b"<=>|";

/// Result of a three-way merge.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Merged {
    Clean(Vec<u8>),
    Conflict(Vec<u8>),
}

/// True when the bytes contain NUL in the first 8000 bytes or are not valid UTF-8.
pub fn is_binary(bytes: &[u8]) -> bool {
    let head = &bytes[..bytes.len().min(BINARY_SCAN_LEN)];
    head.contains(&0) || std::str::from_utf8(bytes).is_err()
}

fn starts_with_run(line: &[u8], ch: u8, len: usize) -> bool {
    line.len() >= len && line[..len].iter().all(|&b| b == ch)
}

fn safe_marker_len(inputs: &[&[u8]]) -> usize {
    let mut len = MIN_MARKER_LEN;
    while inputs.iter().any(|input| {
        input
            .split(|&b| b == b'\n')
            .any(|line| MARKER_CHARS.iter().any(|&c| starts_with_run(line, c, len)))
    }) {
        len += 1;
    }
    len
}

/// Three-way merge of text; callers must check `is_binary` first.
///
/// Conflicts are written as `<<<<<<< project`, `=======`, `>>>>>>> <target_label>`.
pub fn merge3(base: &[u8], project: &[u8], target: &[u8], target_label: &str) -> Merged {
    let len = safe_marker_len(&[base, project, target]);
    let mut options = MergeOptions::new();
    options
        .set_conflict_marker_length(len)
        .set_conflict_style(ConflictStyle::Merge)
        .set_incomplete_hunk_style(IncompleteHunkStyle::Git);
    match options.merge_bytes(base, project, target) {
        Ok(clean) => Merged::Clean(clean),
        Err(raw) => Merged::Conflict(relabel(&raw, len, project, target_label)),
    }
}

/// Rewrites only the marker lines diffy emitted (unambiguous because `len` is safe).
fn relabel(raw: &[u8], len: usize, project: &[u8], target_label: &str) -> Vec<u8> {
    let eol: &[u8] = if project.windows(2).any(|w| w == b"\r\n") {
        b"\r\n"
    } else {
        b"\n"
    };
    let open = [vec![b'<'; len], b" ours\n".to_vec()].concat();
    let sep = [vec![b'='; len], b"\n".to_vec()].concat();
    let close = [vec![b'>'; len], b" theirs\n".to_vec()].concat();
    let mut out = Vec::with_capacity(raw.len() + target_label.len());
    for line in raw.split_inclusive(|&b| b == b'\n') {
        if line == open.as_slice() {
            out.extend_from_slice(b"<<<<<<< project");
        } else if line == sep.as_slice() {
            out.extend_from_slice(b"=======");
        } else if line == close.as_slice() {
            out.extend_from_slice(b">>>>>>> ");
            out.extend_from_slice(target_label.as_bytes());
        } else {
            out.extend_from_slice(line);
            continue;
        }
        out.extend_from_slice(eol);
    }
    out
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

    fn lines(n: usize, edits: &[(usize, &str)]) -> String {
        (1..=n)
            .map(|i| {
                let text = edits
                    .iter()
                    .find(|(k, _)| *k == i)
                    .map_or(format!("line{i}"), |(_, t)| (*t).to_string());
                format!("{text}\n")
            })
            .collect()
    }

    fn conflict(m: Merged) -> String {
        match m {
            Merged::Conflict(b) => String::from_utf8(b).unwrap(),
            Merged::Clean(b) => panic!("expected conflict, got clean {b:?}"),
        }
    }

    #[test]
    fn has_markers_needs_all_three_in_order() {
        assert!(!has_markers(b"title\n=======\nbody\n"));
        assert!(!has_markers(b"<<<<<<< a\nx\n"));
        assert!(!has_markers(b">>>>>>> t\n=======\n<<<<<<< p\n"));
        assert!(has_markers(b"<<<<<<< p\n=======\n>>>>>>> t\n"));
        assert!(has_markers(b"<<<<<<< p\r\n=======\r\n>>>>>>> t\r\n"));
        assert!(!has_markers(b"<<<<<<< p\n=======\n>>>>>>> t\n\0"));
        let merged = conflict(merge3(b"a\n", b"b\n", b"c\n", "template v1.1.0"));
        assert!(has_markers(merged.as_bytes()));
    }

    #[test]
    fn non_overlapping_edits_merge_clean() {
        let base = lines(10, &[]);
        let project = lines(10, &[(2, "mine")]);
        let target = lines(10, &[(9, "theirs")]);
        let want = lines(10, &[(2, "mine"), (9, "theirs")]);
        assert_eq!(
            merge3(base.as_bytes(), project.as_bytes(), target.as_bytes(), "t"),
            Merged::Clean(want.into_bytes())
        );
    }

    #[test]
    fn overlap_gives_markers() {
        let out = conflict(merge3(
            b"a\nb\nc\n",
            b"a\nmine\nc\n",
            b"a\ntheirs\nc\n",
            "template v1.1.0",
        ));
        assert_eq!(
            out,
            "a\n<<<<<<< project\nmine\n=======\ntheirs\n>>>>>>> template v1.1.0\nc\n"
        );
    }

    #[test]
    fn empty_base_identical_is_clean() {
        assert_eq!(
            merge3(b"", b"x\n", b"x\n", "t"),
            Merged::Clean(b"x\n".to_vec())
        );
    }

    #[test]
    fn empty_base_different_conflicts() {
        let out = conflict(merge3(b"", b"x\n", b"y\n", "t"));
        assert_eq!(out, "<<<<<<< project\nx\n=======\ny\n>>>>>>> t\n");
    }

    #[test]
    fn crlf_preserved() {
        let clean = merge3(
            b"a\r\nb\r\nc\r\nd\r\n",
            b"A\r\nb\r\nc\r\nd\r\n",
            b"a\r\nb\r\nc\r\nD\r\n",
            "t",
        );
        assert_eq!(clean, Merged::Clean(b"A\r\nb\r\nc\r\nD\r\n".to_vec()));
        let out = conflict(merge3(b"a\r\nb\r\n", b"a\r\nx\r\n", b"a\r\ny\r\n", "t"));
        assert_eq!(
            out,
            "a\r\n<<<<<<< project\r\nx\r\n=======\r\ny\r\n>>>>>>> t\r\n"
        );
    }

    #[test]
    fn no_final_newline_preserved() {
        let clean = merge3(b"a\nb\nc", b"A\nb\nc", b"a\nb\nC", "t");
        assert_eq!(clean, Merged::Clean(b"A\nb\nC".to_vec()));
        let out = conflict(merge3(b"a\nb", b"a\nx", b"a\ny", "t"));
        assert_eq!(out, "a\n<<<<<<< project\nx\n=======\ny\n>>>>>>> t\n");
    }

    #[test]
    fn marker_lookalike_content_untouched() {
        let base = lines(10, &[]);
        let project = lines(10, &[(2, "<<<<<<< x")]);
        let target = lines(10, &[(9, "theirs")]);
        let want = lines(10, &[(2, "<<<<<<< x"), (9, "theirs")]);
        assert_eq!(
            merge3(base.as_bytes(), project.as_bytes(), target.as_bytes(), "t"),
            Merged::Clean(want.into_bytes())
        );
        let project = "a\n<<<<<<< x\n=======\nc\n";
        let out = conflict(merge3(b"a\nb\nc\n", project.as_bytes(), b"a\nq\nc\n", "t"));
        assert_eq!(out.matches("<<<<<<< project\n").count(), 1);
        assert_eq!(out.matches(">>>>>>> t\n").count(), 1);
        assert!(out.contains("<<<<<<< x\n"));
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
