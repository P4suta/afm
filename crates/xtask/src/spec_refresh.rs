//! Convert a cmark-format `spec.txt` into the JSON fixture the spec
//! conformance tests read.
//!
//! The input convention: a 32-backtick fence followed by the `example`
//! keyword opens a case, a lone `.` separates source from expected HTML, and
//! cases are numbered across the whole file under the last ATX heading seen.
//!
//! The output is canonical-formatted (sorted keys, 2-space indent, trailing
//! newline) so a re-run is byte-identical and CI diffs stay meaningful.

use std::fs;
use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::Serialize;

/// Single spec example, matching comrak's own `spec_runner` shape.
#[derive(Debug, Serialize)]
struct SpecExample {
    pub example: u32,
    pub section: String,
    pub markdown: String,
    pub html: String,
    /// The extension a GFM example exercises. `None` for pure CommonMark.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension: Option<String>,
}

const FENCE: &str = "````````````````````````````````"; // 32 backticks
const FENCE_EXAMPLE_PREFIX: &str = "```````````````````````````````` example";
const SEPARATOR: &str = ".";

#[derive(Debug, PartialEq, Eq)]
enum ExampleOpening {
    Bare,
    Tagged(String),
}

/// `None` if the line does not open an example.
fn parse_example_opening(line: &str) -> Option<ExampleOpening> {
    let rest = line.strip_prefix(FENCE_EXAMPLE_PREFIX)?;
    let tag = rest.trim_start();
    if tag.is_empty() {
        Some(ExampleOpening::Bare)
    } else {
        Some(ExampleOpening::Tagged(tag.to_owned()))
    }
}

/// # Errors
///
/// Unreadable input, malformed format (orphan fence, missing separator,
/// unterminated block), or an unwritable output.
pub(crate) fn refresh_one(input_path: &Path, output_path: &Path) -> Result<usize> {
    let raw = fs::read_to_string(input_path)
        .with_context(|| format!("reading spec source {}", input_path.display()))?;
    let examples =
        parse(&raw).with_context(|| format!("parsing spec source {}", input_path.display()))?;
    let json = serde_json::to_string_pretty(&examples).context("serialising spec examples")?;
    let mut out = json;
    out.push('\n');
    fs::write(output_path, out)
        .with_context(|| format!("writing spec fixture {}", output_path.display()))?;
    Ok(examples.len())
}

/// # Errors
///
/// An `example` fence that never sees its separator or closing fence, or a
/// closing fence with no opening one.
fn parse(source: &str) -> Result<Vec<SpecExample>> {
    let mut out = Vec::new();
    let mut section = String::new();
    let mut example = 0u32;

    let mut state = State::Outside;
    let mut md_buf = String::new();
    let mut html_buf = String::new();
    let mut current_extension: Option<String> = None;

    for (lineno, line) in source.lines().enumerate() {
        let lineno = lineno + 1; // 1-indexed for diagnostics

        match &mut state {
            State::Outside => {
                if let Some(rest) = line.strip_prefix("# ") {
                    rest.trim().clone_into(&mut section);
                    continue;
                }
                if let Some(opening) = parse_example_opening(line) {
                    example += 1;
                    md_buf.clear();
                    html_buf.clear();
                    current_extension = match opening {
                        ExampleOpening::Bare => None,
                        ExampleOpening::Tagged(tag) => Some(tag),
                    };
                    state = State::Markdown;
                    continue;
                }
                // Guard against stray closing fences.
                if line == FENCE {
                    bail!(
                        "spec parse error at line {lineno}: closing fence `{FENCE}` \
                         outside any example block"
                    );
                }
            }
            State::Markdown => {
                if line == SEPARATOR {
                    state = State::Html;
                    continue;
                }
                md_buf.push_str(line);
                md_buf.push('\n');
            }
            State::Html => {
                if line == FENCE {
                    let md = normalise(&md_buf);
                    let html = normalise(&html_buf);
                    out.push(SpecExample {
                        example,
                        section: section.clone(),
                        markdown: md,
                        html,
                        extension: current_extension.take(),
                    });
                    md_buf.clear();
                    html_buf.clear();
                    state = State::Outside;
                    continue;
                }
                html_buf.push_str(line);
                html_buf.push('\n');
            }
        }
    }

    if !matches!(state, State::Outside) {
        bail!(
            "spec parse error: example {example} (section {section:?}) \
             ended mid-block — expected closing fence"
        );
    }
    Ok(out)
}

#[derive(Debug, Clone, Copy)]
enum State {
    Outside,
    Markdown,
    Html,
}

/// cmark encodes two ASCII-invisible chars in its spec fixtures:
///   →  (U+2192) represents a literal TAB (U+0009)
///   ·  — not used in CommonMark 0.31.2 body text; the spec uses → only, but
///        we normalise both for compatibility with future cmark-gfm files.
fn normalise(s: &str) -> String {
    s.replace('\u{2192}', "\t")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env;
    use std::io::Write;
    use std::path::PathBuf;
    use std::process;
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn parses_single_example() {
        let input = format!(
            "# Tabs\n\n{FENCE_EXAMPLE_PREFIX}\n\u{2192}foo\n.\n<pre><code>\tfoo\n</code></pre>\n{FENCE}\n"
        );
        let out = parse(&input).expect("parse");
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].example, 1);
        assert_eq!(out[0].section, "Tabs");
        assert_eq!(out[0].markdown, "\tfoo\n");
        assert_eq!(out[0].html, "<pre><code>\tfoo\n</code></pre>\n");
    }

    #[test]
    fn tracks_sections_across_examples() {
        let input = format!(
            "# First\n\n{FENCE_EXAMPLE_PREFIX}\na\n.\n<p>a</p>\n{FENCE}\n\n# Second\n\n{FENCE_EXAMPLE_PREFIX}\nb\n.\n<p>b</p>\n{FENCE}\n"
        );
        let out = parse(&input).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[0].section, "First");
        assert_eq!(out[1].section, "Second");
        assert_eq!(out[1].example, 2);
    }

    #[test]
    fn fails_on_unterminated_example() {
        let input = format!("{FENCE_EXAMPLE_PREFIX}\nunterminated\n");
        let err = parse(&input).unwrap_err().to_string();
        assert!(err.contains("ended mid-block"), "error was: {err}");
    }

    #[test]
    fn fails_on_stray_closing_fence() {
        let input = format!("# x\n{FENCE}\n");
        let err = parse(&input).unwrap_err().to_string();
        assert!(err.contains("closing fence"), "error was: {err}");
    }

    #[test]
    fn refresh_one_writes_canonical_json() {
        let dir = tempdir();
        let src = dir.join("spec.txt");
        let dst = dir.join("spec.json");
        let mut f = fs::File::create(&src).unwrap();
        writeln!(f, "# Tabs").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "{FENCE_EXAMPLE_PREFIX}").unwrap();
        writeln!(f, "foo").unwrap();
        writeln!(f, ".").unwrap();
        writeln!(f, "<p>foo</p>").unwrap();
        writeln!(f, "{FENCE}").unwrap();

        let n = refresh_one(&src, &dst).expect("refresh");
        assert_eq!(n, 1);
        let written = fs::read_to_string(&dst).unwrap();
        assert!(written.starts_with('['));
        assert!(written.ends_with("]\n"));
        assert!(written.contains(r#""markdown": "foo\n""#));
    }

    // Minimal tempdir helper — avoids pulling in the `tempfile` crate for a
    // two-test use case. Builds a per-test dir under `env::temp_dir()`.
    fn tempdir() -> PathBuf {
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let id = SEQ.fetch_add(1, Ordering::SeqCst);
        let pid = process::id();
        let dir = env::temp_dir().join(format!("aozora-md-xtask-{pid}-{id}"));
        fs::create_dir_all(&dir).unwrap();
        dir
    }
}
