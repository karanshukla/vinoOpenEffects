//! Every test a comment cites as ``(pinned by `tests::name`)`` must be a
//! `#[test]` fn, so a rule that names its test fails CI when the test is
//! renamed or deleted. rustdoc can't check these: `#[test]` fns are compiled
//! out of the docs. Scans every workspace crate. See docs/comment-style.md.

use std::fs;
use std::path::{Path, PathBuf};

/// The phrase that introduces citations; change it here and in the docs.
const MARKER: &str = "pinned by";

/// Crates whose `src/` is scanned, relative to the workspace root.
const CRATES: &[&str] = &["shared", "core", "daemon", "cli", "gui"];

#[test]
fn every_cited_test_exists() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap();
    let mut broken = Vec::new();
    let mut found = 0;
    for krate in CRATES {
        let src = root.join(krate).join("src");
        for file in rust_files(&src) {
            let text = fs::read_to_string(&file).unwrap();
            for (line_no, cited) in citations(&text) {
                found += 1;
                if !resolves(&src, &file, cited) {
                    let shown = file.strip_prefix(root).unwrap().display();
                    broken.push(format!("{shown}:{line_no}: `{cited}`"));
                }
            }
        }
    }
    assert!(found > 0, "no citations found; is MARKER right?");
    assert!(broken.is_empty(), "no such test:\n{}", broken.join("\n"));
}

#[test]
fn the_check_finds_a_cited_test_and_rejects_a_missing_one() {
    let text = "/// (pinned by `tests::real`,\n/// `tests::gone`)\n#[cfg(test)]\nmod tests {\n    #[test]\n    fn real() {}\n    fn helper() {}\n}\n";
    let cited: Vec<_> = citations(text).into_iter().map(|(_, c)| c).collect();
    assert_eq!(cited, ["tests::real", "tests::gone"]);
    assert!(is_test_fn(text, "real"));
    assert!(!is_test_fn(text, "helper"), "not a #[test]");
    assert!(!is_test_fn(text, "gone"));
}

#[test]
fn a_code_span_outside_a_citation_is_not_checked() {
    let text = "// `tests::anything` is mentioned, not cited\nfn f() {}\n";
    assert!(citations(text).is_empty());
}

fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(dir).unwrap().flatten() {
        let path = entry.path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|e| e == "rs") {
            files.push(path);
        }
    }
    files
}

/// Each cited path with its 1-based line: the code spans from a comment line
/// containing `MARKER` up to the `)` that closes the citation, which may
/// continue over the following comment lines.
fn citations(text: &str) -> Vec<(usize, &str)> {
    let mut out = Vec::new();
    let mut open = false;
    for (i, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if !trimmed.starts_with("//") {
            open = false;
            continue;
        }
        let rest = match trimmed.find(MARKER) {
            Some(at) => {
                open = true;
                &trimmed[at + MARKER.len()..]
            }
            None if open => trimmed,
            None => continue,
        };
        for (n, piece) in rest.split('`').enumerate() {
            if n % 2 == 0 {
                if piece.contains(')') {
                    open = false;
                    break;
                }
            } else if is_test_path(piece) {
                out.push((i + 1, piece));
            }
        }
    }
    out
}

fn is_test_path(span: &str) -> bool {
    span.contains("tests::")
        && span
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
}

/// `tests::name` (or `golden_tests::name`): a child module of the citing
/// file's module, inline or in its own file. `a::b::tests::name`: the same,
/// from the crate root.
fn resolves(src: &Path, file: &Path, cited: &str) -> bool {
    let mut parts: Vec<&str> = cited.split("::").collect();
    let name = parts.pop().unwrap();
    let child = parts.pop().unwrap();
    let parent_file = if parts.is_empty() {
        file.to_path_buf()
    } else {
        let module = src.join(parts.join("/"));
        let flat = module.with_extension("rs");
        if flat.exists() {
            flat
        } else {
            module.join("mod.rs")
        }
    };
    let parent_dir = if parent_file.ends_with("mod.rs")
        || parent_file.ends_with("lib.rs")
        || parent_file.ends_with("main.rs")
    {
        parent_file.parent().unwrap().to_path_buf()
    } else {
        parent_file.with_extension("")
    };
    [parent_file, parent_dir.join(format!("{child}.rs"))]
        .iter()
        .filter_map(|f| fs::read_to_string(f).ok())
        .any(|text| is_test_fn(&text, name))
}

/// A `fn name(` whose attributes, above it, include `#[test]` or `#[tokio::test]`.
fn is_test_fn(text: &str, name: &str) -> bool {
    let lines: Vec<&str> = text.lines().collect();
    let signature = format!("fn {name}(");
    lines.iter().enumerate().any(|(i, line)| {
        let line = line.trim_start();
        (line.starts_with(&signature) || line.starts_with(&format!("async {signature}")))
            && lines[..i]
                .iter()
                .rev()
                .map(|l| l.trim())
                .take_while(|l| l.starts_with("#[") || l.starts_with("///"))
                .any(|l| l == "#[test]" || l.starts_with("#[tokio::test"))
    })
}
