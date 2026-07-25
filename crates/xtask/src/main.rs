//! Workspace automation.
//!
//! Every task invoked by the `Justfile` or by CI that isn't a direct cargo
//! invocation lives here. Sub-commands:
//!
//! - `upstream-diff` — assert the vendored comrak tree is still
//!   pinned to the recorded SHA and that the ADR-0001 0-line diff
//!   budget is documented in `upstream/comrak/UPSTREAM_DIFF.md`.
//! - `upstream-sync` — replace `upstream/comrak/` with the source
//!   tree at a given upstream tag. Pure tree-replace (ADR-0001): the
//!   diff budget is 0, so there are no patches to re-apply.
//! - `comment-discipline` — fail when a Rust or TOML comment names a
//!   retired upstream-internal path (ADR-0021).
//! - `new-adr` — scaffold a new MADR file under `docs/adr/`.
//! - `spec-refresh` — regenerate `spec/commonmark-*.json` /
//!   `spec/gfm-*.json` from cmark-format `spec.txt` inputs. Network
//!   fetching is handled by the `just spec-refresh` target
//!   (shell-side `curl`); this xtask only transforms
//!   already-downloaded spec files into fixture JSON.
//!
//! Aozora parser / corpus concerns live in the sibling `P4suta/aozora`
//! repo (ADR-0010), along with their refresh sub-commands.

#![forbid(unsafe_code)]

use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

mod spec_refresh;

/// ADR-0001 upstream diff budget, in lines. The vendored tree is verbatim
/// (no hooks), so the budget is 0; changing it requires a new ADR.
const UPSTREAM_DIFF_BUDGET_LINES: usize = 0;

/// Upstream comrak repository URL. `upstream-sync` shallow-clones a
/// single tag from this remote.
const UPSTREAM_COMRAK_URL: &str = "https://github.com/kivikakk/comrak.git";

#[derive(Parser, Debug)]
#[command(version, about = "aozora-flavored-markdown workspace automation", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Verify the ADR-0001 upstream-diff policy is in force.
    UpstreamDiff,
    /// Replace `upstream/comrak/` with the source tree at the given
    /// upstream tag. Pure tree-replace (ADR-0001).
    UpstreamSync {
        /// Upstream tag name (e.g. `v0.53.0`).
        tag: String,
    },
    /// Fail if any comment under `crates/` names a retired
    /// upstream-internal path. The boundary with the sibling parser is its
    /// public API only (ADR-0021), and prose must not outlive it.
    CommentDiscipline,
    /// Create a new Architecture Decision Record under `docs/adr/`.
    NewAdr { title: String },
    /// Convert cmark-format spec.txt inputs to fixture JSON. Pass one or more
    /// `--from <input>=<output>` pairs. Each pair rewrites one JSON fixture.
    SpecRefresh {
        /// Input spec source file (plain text, cmark fenced-example format).
        #[arg(long)]
        input: PathBuf,
        /// Output JSON fixture path.
        #[arg(long)]
        output: PathBuf,
    },
    /// Bump the published `aozora` version across the workspace manifest
    /// (the umbrella `aozora` dep) and the cargo-fuzz crate in one pass,
    /// then refresh `Cargo.lock`. Keeping both pins on the same version
    /// stops a sync from silently leaving the fuzz crate behind the
    /// workspace.
    AozoraBump {
        /// Published `major.minor.patch` version from crates.io.
        version: String,
    },
    /// Generate (or, with `--check`, drift-check) the release assets bundled
    /// into the dist archives: shell completions and the man page, written
    /// under `dist/assets/`. Shells out to the built `aozora-flavored-markdown` binary so the CLI
    /// definition stays the single source of truth.
    GenDistAssets {
        /// Compare committed assets against fresh generation and exit non-zero
        /// on drift, instead of rewriting them.
        #[arg(long)]
        check: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
        Command::UpstreamDiff => upstream_diff(),
        Command::UpstreamSync { tag } => upstream_sync(&tag),
        Command::CommentDiscipline => comment_discipline(Path::new(".")),
        Command::NewAdr { title } => new_adr(&title),
        Command::SpecRefresh { input, output } => {
            let n = spec_refresh::refresh_one(&input, &output).with_context(|| {
                format!(
                    "refreshing spec {} -> {}",
                    input.display(),
                    output.display()
                )
            })?;
            println!("spec-refresh: wrote {n} examples to {}", output.display());
            Ok(())
        }
        Command::AozoraBump { version } => aozora_bump(&version),
        Command::GenDistAssets { check } => gen_dist_assets(check),
    }
}

/// Shells (`clap_complete`) we ship completions for, with their conventional
/// install filenames.
const COMPLETION_TARGETS: [(&str, &str); 5] = [
    ("bash", "aozora-flavored-markdown.bash"),
    ("zsh", "_aozora-flavored-markdown"),
    ("fish", "aozora-flavored-markdown.fish"),
    ("powershell", "_aozora-flavored-markdown.ps1"),
    ("elvish", "aozora-flavored-markdown.elv"),
];

/// Generate, or drift-check, the completion scripts and man page bundled into
/// the release archives. Generation runs the built `aozora-flavored-markdown` binary so the CLI
/// definition is the single source of truth (aozora-flavored-markdown-cli is a binary, not a
/// library, so xtask cannot import its `Cli` directly).
fn gen_dist_assets(check: bool) -> Result<()> {
    let bin = aozora_binary_path();
    if !bin.is_file() {
        bail!(
            "gen-dist-assets: {} not found — build it first (`cargo build -p aozora-flavored-markdown-cli`); \
             `just dist-assets` does this for you",
            bin.display()
        );
    }

    let comp_dir = PathBuf::from("dist/assets/completions");
    let man_path = PathBuf::from("dist/assets/man/aozora-flavored-markdown.1");
    let mut drift: Vec<String> = Vec::new();

    for (shell, filename) in COMPLETION_TARGETS {
        let script = run_cli_capture(&bin, &["completions", shell])?;
        sync_or_check(&comp_dir.join(filename), &script, check, &mut drift)?;
    }
    let man = run_cli_capture(&bin, &["_man"])?;
    sync_or_check(&man_path, &man, check, &mut drift)?;

    if check {
        if drift.is_empty() {
            println!("gen-dist-assets: committed assets are up to date");
            Ok(())
        } else {
            bail!(
                "gen-dist-assets: {} asset(s) out of date ({}). \
                 Run `just dist-assets` and commit the result.",
                drift.len(),
                drift.join(", ")
            )
        }
    } else {
        println!(
            "gen-dist-assets: wrote {} completion script(s) + man page under dist/assets/",
            COMPLETION_TARGETS.len()
        );
        Ok(())
    }
}

/// Path to the debug `aozora-flavored-markdown` binary, honoring `CARGO_TARGET_DIR`.
fn aozora_binary_path() -> PathBuf {
    let target =
        env::var_os("CARGO_TARGET_DIR").map_or_else(|| PathBuf::from("target"), PathBuf::from);
    target.join("debug").join("aozora-flavored-markdown")
}

/// Run the built `aozora-flavored-markdown` binary with `args` and return its stdout, or bail on a
/// non-zero exit.
fn run_cli_capture(bin: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = ProcessCommand::new(bin)
        .args(args)
        .output()
        .with_context(|| format!("running {} {args:?}", bin.display()))?;
    if !out.status.success() {
        bail!(
            "{} {args:?} failed: {}",
            bin.display(),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(out.stdout)
}

/// Write `content` to `dest` (creating parents), or in check mode record `dest`
/// as drifted when it differs.
fn sync_or_check(dest: &Path, content: &[u8], check: bool, drift: &mut Vec<String>) -> Result<()> {
    if check {
        let existing = fs::read(dest).unwrap_or_default();
        if existing != content {
            drift.push(dest.display().to_string());
        }
    } else {
        if let Some(parent) = dest.parent() {
            fs::create_dir_all(parent).with_context(|| format!("mkdir {}", parent.display()))?;
        }
        fs::write(dest, content).with_context(|| format!("writing {}", dest.display()))?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// comment-discipline
// ---------------------------------------------------------------------------

/// Retired upstream-internal names, each with the reason it is banned.
///
/// The sibling parser publishes a small, deliberately curated API; everything
/// listed here was an *internal* path this workspace once reached into and
/// which no longer exists. A comment naming one is prose that has already
/// rotted — it teaches a reader an import that cannot compile. The remedy is
/// always the same: describe the behaviour, not the upstream internal that
/// provides it (ADR-0021).
///
/// Entries are written in their manifest spelling; [`fold_separators`] makes
/// each one match the `_` spelling too, so one row covers both.
///
/// Real code is out of scope: code that stops compiling is caught by the
/// compiler, and this gate exists precisely for the class of drift the
/// compiler cannot see.
const RETIRED_UPSTREAM_PATHS: &[(&str, &str)] = &[
    (
        "aozora::pipeline",
        "the pipeline module is private upstream",
    ),
    ("aozora::syntax", "the syntax module is private upstream"),
    ("aozora::render", "the render module is private upstream"),
    (
        "aozora::encoding",
        "the encoding module is private upstream",
    ),
    ("aozora-pipeline", "no longer a published crate"),
    ("aozora-syntax", "no longer a published crate"),
    ("aozora-render", "no longer a published crate"),
    ("aozora-encoding", "no longer a published crate"),
    ("aozora-proptest", "no longer a published crate"),
    ("aozora-spec", "no longer a published crate"),
    ("aozora-corpus", "no longer a published crate"),
    ("aozora-lexer", "no longer a published crate"),
    ("aozora-parser", "no longer a published crate"),
    ("aozora-scan", "no longer a published crate"),
    ("lex_into_arena", "not on the public surface"),
    ("BorrowedLexOutput", "not on the public surface"),
    ("NormalizedOffset", "not on the public surface"),
    ("node_at_normalized", "removed upstream"),
    ("render_node", "not on the public surface"),
    ("AozoraNode", "not on the public surface"),
];

/// One comment line that names a retired upstream path.
#[derive(Debug, PartialEq, Eq)]
struct CommentViolation {
    line: usize,
    needle: &'static str,
    why: &'static str,
    text: String,
}

/// Fold `_` into `-` so one banned entry matches either spelling of a name.
///
/// Rust writes the same crate two ways: hyphenated in a manifest, underscored
/// in an intra-doc link. A list that knows only the manifest spelling misses
/// every rustdoc reference — which is exactly where this drift accumulates,
/// because an intra-doc link to a dependency that has gone away is what turns
/// `just doc` red. Folding both the needle and the comment makes the two
/// spellings indistinguishable to the gate.
fn fold_separators(text: &str) -> String {
    text.replace('_', "-")
}

/// File kinds this gate reads, each with the token its comments open on.
///
/// Rust and TOML are where a retired upstream name is written in prose that
/// nothing compiles: a doc comment and a manifest note. A stale manifest note
/// rots exactly like a stale doc comment — it is where the reason for a lint
/// setting or a feature flag is recorded — so it belongs under the same gate.
/// Markdown is deliberately out: `CHANGELOG.md` records what the retired
/// crates *were*, and history is not drift.
const SCANNED_FILES: &[(&str, &str)] = &[("rs", "//"), ("toml", "#")];

/// Directories that are never authored here.
///
/// `target/` is cargo output (including each fuzz crate's own), `pkg/` is
/// wasm-pack output, `node_modules/` is bun's. `upstream/` is the verbatim
/// vendored comrak tree, which ADR-0001 budgets zero edits for — gating prose
/// nobody may rewrite would only be a trap.
const UNSCANNED_DIRS: &[&str] = &["target", "pkg", "node_modules", "upstream", ".git"];

/// Return every comment line in `src` that names a retired upstream path.
///
/// `marker` is the file kind's comment opener, per [`SCANNED_FILES`]; for
/// Rust that one token covers `///`, `//!` and plain `//` alike. Prose rots
/// the same way whichever marker introduces it, and the compiler already owns
/// everything else.
fn scan_comments(src: &str, marker: &str) -> Vec<CommentViolation> {
    let banned: Vec<(String, &str, &str)> = RETIRED_UPSTREAM_PATHS
        .iter()
        .map(|&(needle, why)| (fold_separators(needle), needle, why))
        .collect();

    let mut out = Vec::new();
    for (idx, raw) in src.lines().enumerate() {
        let trimmed = raw.trim_start();
        if !trimmed.starts_with(marker) {
            continue;
        }
        let folded = fold_separators(trimmed);
        for (folded_needle, needle, why) in &banned {
            if folded.contains(folded_needle.as_str()) {
                out.push(CommentViolation {
                    line: idx + 1,
                    needle,
                    why,
                    text: trimmed.to_owned(),
                });
            }
        }
    }
    out
}

/// Collect every scannable file under `dir` with the comment marker its kind
/// uses, skipping the directories nobody here authors.
fn collect_scannable_files(dir: &Path, out: &mut Vec<(PathBuf, &'static str)>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading an entry of {}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name.to_str().is_some_and(|n| UNSCANNED_DIRS.contains(&n)) {
                continue;
            }
            collect_scannable_files(&path, out)?;
        } else if let Some(marker) = path
            .extension()
            .and_then(|e| e.to_str())
            .and_then(|ext| SCANNED_FILES.iter().find(|&&(kind, _)| kind == ext))
            .map(|&(_, marker)| marker)
        {
            out.push((path, marker));
        }
    }
    out.sort();
    Ok(())
}

/// Fail when a comment under `root` names a retired upstream path.
fn comment_discipline(root: &Path) -> Result<()> {
    if !root.is_dir() {
        bail!(
            "comment-discipline: {} not found; run from the workspace root",
            root.display()
        );
    }

    let mut files = Vec::new();
    collect_scannable_files(root, &mut files)?;

    let mut total = 0usize;
    for (path, marker) in &files {
        let src =
            fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        for v in scan_comments(&src, marker) {
            if total == 0 {
                println!("comment-discipline: comments naming retired upstream paths:");
            }
            total += 1;
            println!(
                "  {}:{}: `{}` ({})\n    {}",
                path.display(),
                v.line,
                v.needle,
                v.why,
                v.text
            );
        }
    }

    if total > 0 {
        bail!(
            "comment-discipline: {total} comment reference(s) to retired upstream paths. \
             The boundary is the sibling parser's public API only (ADR-0021) — describe the \
             behaviour instead of naming an upstream internal."
        );
    }

    println!(
        "comment-discipline: clean ({} file(s), {} banned path(s) checked)",
        files.len(),
        RETIRED_UPSTREAM_PATHS.len()
    );
    Ok(())
}

/// Verify the ADR-0001 upstream-diff policy is in force.
///
/// Reads the pinned SHA + tag from `upstream/comrak/COMRAK_SHA` and
/// asserts that `upstream/comrak/UPSTREAM_DIFF.md` mentions the
/// current budget number ([`UPSTREAM_DIFF_BUDGET_LINES`]).
///
/// Byte-level enforcement against the upstream remote (network
/// fetch + diff) is **not** part of this gate: developers run
/// `cargo xtask upstream-sync <tag>` (a pure tree replace per ADR-0001)
/// to refresh the vendored tree, and any local modification
/// has to pass code review. The gate here catches accidental drift
/// in the policy file itself.
fn upstream_diff() -> Result<()> {
    let sha_path = PathBuf::from("upstream/comrak/COMRAK_SHA");
    let raw =
        fs::read_to_string(&sha_path).with_context(|| format!("reading {}", sha_path.display()))?;

    // COMRAK_SHA is two lines: the pinned commit SHA on line 1, the
    // upstream tag name (e.g. "v0.52.0") on line 2. Both are optional to
    // mention in the output but the SHA is required for the gate.
    let mut lines = raw.lines().map(str::trim).filter(|s| !s.is_empty());
    let sha = lines.next().unwrap_or_default();
    let tag = lines.next().unwrap_or_default();
    if sha.is_empty() {
        bail!("{} is empty", sha_path.display());
    }

    let diff_md_path = PathBuf::from("upstream/comrak/UPSTREAM_DIFF.md");
    let diff_md = fs::read_to_string(&diff_md_path)
        .with_context(|| format!("reading {}", diff_md_path.display()))?;

    // We want the budget number to appear in a phrase that names a
    // line count, not as a stray digit. `<n>-line` and `<n> lines`
    // are the two phrasings UPSTREAM_DIFF.md uses; either is enough.
    let needle_hyphen = format!("{UPSTREAM_DIFF_BUDGET_LINES}-line");
    let needle_word = format!("{UPSTREAM_DIFF_BUDGET_LINES} lines");
    if !diff_md.contains(&needle_hyphen) && !diff_md.contains(&needle_word) {
        bail!(
            "{} does not mention the {}-line upstream diff budget (ADR-0001)",
            diff_md_path.display(),
            UPSTREAM_DIFF_BUDGET_LINES,
        );
    }

    if tag.is_empty() {
        println!("upstream-diff: vendored comrak pinned at {sha}");
    } else {
        println!("upstream-diff: vendored comrak pinned at {sha} ({tag})");
    }
    println!(
        "upstream-diff: budget {UPSTREAM_DIFF_BUDGET_LINES} lines (ADR-0001), policy documented in {}",
        diff_md_path.display()
    );

    Ok(())
}

/// Replace `upstream/comrak/` with the source tree at `tag`.
///
/// Pure tree-replace (ADR-0001): there are no aozora-flavored-markdown patches to re-apply
/// because the diff budget is 0. We preserve the two
/// aozora-md-side metadata files (`COMRAK_SHA` and `UPSTREAM_DIFF.md`)
/// across the wipe, then rewrite `COMRAK_SHA` with the new pin.
///
/// Network: shells out to `git clone --depth 1 --branch <tag>`.
/// Run from a developer machine with internet access; CI does not
/// invoke this command.
fn upstream_sync(tag: &str) -> Result<()> {
    let upstream_dir = PathBuf::from("upstream/comrak");
    if !upstream_dir.is_dir() {
        bail!(
            "upstream-sync: {} not found; run from the workspace root",
            upstream_dir.display()
        );
    }

    let sha_path = upstream_dir.join("COMRAK_SHA");
    let diff_md_path = upstream_dir.join("UPSTREAM_DIFF.md");
    let preserved: Vec<(PathBuf, Vec<u8>)> = [&sha_path, &diff_md_path]
        .into_iter()
        .filter_map(|p| fs::read(p).ok().map(|c| (p.clone(), c)))
        .collect();

    let scratch = PathBuf::from("target/upstream-sync-tmp");
    if scratch.exists() {
        fs::remove_dir_all(&scratch)
            .with_context(|| format!("removing stale {}", scratch.display()))?;
    }
    if let Some(parent) = scratch.parent() {
        fs::create_dir_all(parent).with_context(|| format!("ensuring {}", parent.display()))?;
    }

    let status = ProcessCommand::new("git")
        .args([
            "clone",
            "--depth",
            "1",
            "--branch",
            tag,
            UPSTREAM_COMRAK_URL,
        ])
        .arg(&scratch)
        .status()
        .context("running `git clone`")?;
    if !status.success() {
        bail!("git clone failed for tag {tag:?}");
    }

    let sha_out = ProcessCommand::new("git")
        .arg("-C")
        .arg(&scratch)
        .args(["rev-parse", "HEAD"])
        .output()
        .context("running `git rev-parse HEAD`")?;
    if !sha_out.status.success() {
        bail!(
            "git rev-parse failed: {}",
            String::from_utf8_lossy(&sha_out.stderr)
        );
    }
    let sha = String::from_utf8(sha_out.stdout)
        .context("git rev-parse output not UTF-8")?
        .trim()
        .to_owned();

    // Drop the .git/ directory — we vendor the source tree, not a
    // working clone.
    let dot_git = scratch.join(".git");
    if dot_git.exists() {
        fs::remove_dir_all(&dot_git).with_context(|| format!("removing {}", dot_git.display()))?;
    }

    // Wipe and replace.
    fs::remove_dir_all(&upstream_dir)
        .with_context(|| format!("removing {}", upstream_dir.display()))?;
    copy_dir_recursive(&scratch, &upstream_dir)
        .with_context(|| format!("copying scratch tree into {}", upstream_dir.display()))?;

    // Restore aozora-flavored-markdown metadata, then update COMRAK_SHA with the new pin.
    for (path, content) in preserved {
        fs::write(&path, content).with_context(|| format!("restoring {}", path.display()))?;
    }
    fs::write(&sha_path, format!("{sha}\n{tag}\n"))
        .with_context(|| format!("writing {}", sha_path.display()))?;

    fs::remove_dir_all(&scratch).with_context(|| format!("cleaning {}", scratch.display()))?;

    println!("upstream-sync: replaced upstream/comrak/ with comrak {tag} ({sha})");
    println!("upstream-sync: review the diff and run `just ci` before committing");
    Ok(())
}

/// Copy `src/` into `dst/` recursively. Mirrors the subset of
/// behaviour we need from a real `cp -R` for vendored source trees:
/// regular files are copied byte-for-byte, directories are
/// reconstructed, and symlinks fail loudly (comrak's tree has none,
/// and silently dropping them would break a future upstream change
/// without warning).
fn copy_dir_recursive(src: &Path, dst: &Path) -> Result<()> {
    fs::create_dir_all(dst).with_context(|| format!("mkdir {}", dst.display()))?;
    for entry in fs::read_dir(src).with_context(|| format!("read_dir {}", src.display()))? {
        let entry = entry?;
        let ty = entry.file_type()?;
        let from = entry.path();
        let to = dst.join(entry.file_name());
        if ty.is_dir() {
            copy_dir_recursive(&from, &to)?;
        } else if ty.is_symlink() {
            bail!(
                "unsupported symlink at {}; upstream comrak should only contain \
                 regular files and directories",
                from.display()
            );
        } else {
            // Regular file (or platform-specific kind we treat as a
            // file). `is_file()` would be too narrow on some
            // filesystems; `!is_dir() && !is_symlink()` handles
            // hardlinked entries that `fs::copy` accepts.
            fs::copy(&from, &to)
                .with_context(|| format!("copy {} -> {}", from.display(), to.display()))?;
        }
    }
    Ok(())
}

/// Scaffold a new Architecture Decision Record under `docs/adr/`.
///
/// Picks the next available four-digit prefix and writes a minimal MADR
/// template. `slugify` normalises the user-supplied title to a filename
/// form; collisions against existing ADRs fail loudly rather than
/// silently overwriting.
fn new_adr(title: &str) -> Result<()> {
    let adr_dir = PathBuf::from("docs/adr");
    if !adr_dir.is_dir() {
        bail!("ADR directory not found at {}", adr_dir.display());
    }

    let mut max_num: u32 = 0;
    for entry in fs::read_dir(&adr_dir).with_context(|| format!("reading {}", adr_dir.display()))? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if let Some(num_str) = name.split('-').next()
            && let Ok(n) = num_str.parse::<u32>()
        {
            max_num = max_num.max(n);
        }
    }

    let next_num = max_num + 1;
    let slug = slugify(title);
    if slug.is_empty() {
        bail!("title {title:?} produced an empty slug");
    }
    let filename = format!("{next_num:04}-{slug}.md");
    let path = adr_dir.join(&filename);

    if path.exists() {
        bail!("ADR already exists at {}", path.display());
    }

    let today = today_yyyy_mm_dd()?;
    // Render `0000-template.md` rather than hard-coding a divergent subset, so
    // a scaffolded ADR carries the same section set (Status / Date / Deciders /
    // Tags + Context / Decision / Consequences / Alternatives / References) the
    // committed ADRs use. The author fills Tags and the section bodies.
    let template_path = adr_dir.join("0000-template.md");
    let template = fs::read_to_string(&template_path)
        .with_context(|| format!("reading ADR template {}", template_path.display()))?;
    let content = template
        .replace("{{ADR NUMBER}}", &format!("{next_num:04}"))
        .replace("{{TITLE}}", title)
        .replace(
            "{proposed | accepted | deprecated | superseded by ADR-XXXX}",
            "proposed",
        )
        .replace("YYYY-MM-DD", &today);

    fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
    println!("created {}", path.display());
    Ok(())
}

fn slugify(title: &str) -> String {
    let mut out = String::new();
    let mut prev_dash = false;
    for ch in title.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
    }
    out.trim_end_matches('-').to_owned()
}

fn today_yyyy_mm_dd() -> Result<String> {
    let out = ProcessCommand::new("date")
        .arg("+%Y-%m-%d")
        .output()
        .context("invoking `date` to stamp the new ADR")?;
    if !out.status.success() {
        bail!(
            "date command failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8(out.stdout)
        .context("date output was not valid UTF-8")?
        .trim()
        .to_owned())
}

/// Manifests carrying an `aozora` version pin. The workspace manifest
/// declares the umbrella crate for every member; the workspace-external
/// cargo-fuzz crate declares the same version independently.
/// `aozora-bump` rewrites both in one pass so a sync can't leave the fuzz
/// crate behind, then refreshes Cargo.lock. Idempotent: if every pin
/// already matches the target version, nothing is written and
/// `cargo update` is skipped.
const AOZORA_PINNED_MANIFESTS: [&str; 2] = [
    "Cargo.toml",
    "crates/aozora-flavored-markdown/fuzz/Cargo.toml",
];

/// Rewrite the `aozora` version pin in [`AOZORA_PINNED_MANIFESTS`].
///
/// The `=` an exact-version requirement carries is preserved, so the fuzz
/// crate keeps pinning the workspace's version exactly while the workspace
/// itself stays on a caret requirement.
fn aozora_pin_pattern() -> regex::Regex {
    regex::Regex::new(r#"(?m)^(aozora\s*=\s*\{\s*version\s*=\s*"=?)(\d+\.\d+\.\d+)(")"#)
        .expect("aozora version pattern compiles")
}

fn aozora_bump(new_version: &str) -> Result<()> {
    // Accept only a fully-spelled `major.minor.patch` — a bare `0.5` would
    // resolve fine but leaves the two manifests textually different and the
    // `verify-version-pins` gate unable to compare them.
    if !is_semver_triple(new_version) {
        bail!("aozora-bump: version must be a full major.minor.patch triple, got: {new_version:?}");
    }

    let pattern = aozora_pin_pattern();
    let mut total_found = 0_usize;
    let mut rewritten = 0_usize;
    for manifest in AOZORA_PINNED_MANIFESTS {
        let path = PathBuf::from(manifest);
        let original =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

        let mut found = 0_usize;
        let mut already = 0_usize;
        let updated = pattern.replace_all(&original, |caps: &regex::Captures<'_>| {
            found += 1;
            if &caps[2] == new_version {
                already += 1;
            }
            format!("{}{new_version}{}", &caps[1], &caps[3])
        });

        // Each manifest pins the umbrella `aozora` exactly once. A
        // different count means the dep block was refactored — bail rather
        // than rewrite an unexpected shape and leave Cargo.lock inconsistent.
        if found != 1 {
            bail!(
                "aozora-bump: expected exactly one `aozora` version pin in {}, \
                 found {found}. The aozora dependency may have been refactored — update \
                 `AOZORA_PINNED_MANIFESTS` / the regex in xtask/src/main.rs and re-run.",
                path.display(),
            );
        }
        total_found += found;
        if already != found {
            fs::write(&path, updated.as_ref())
                .with_context(|| format!("writing {}", path.display()))?;
            rewritten += 1;
            println!(
                "aozora-bump: rewrote {} to version = {new_version}",
                path.display()
            );
        }
    }

    if rewritten == 0 {
        println!("aozora-bump: all {total_found} pins already at {new_version}; no change.");
        return Ok(());
    }

    // Refresh the workspace Cargo.lock. The cargo-fuzz crate's lock is
    // git-ignored (regenerated on the next `cargo fuzz` build), so only the
    // umbrella `aozora` in the workspace needs an explicit update here.
    let status = ProcessCommand::new("cargo")
        .args(["update", "-p", "aozora"])
        .status()
        .context("invoking `cargo update -p aozora`")?;
    if !status.success() {
        bail!(
            "cargo update exited with {status:?}. The manifests were rewritten — \
             re-run `cargo update -p aozora` manually after fixing the fetch / \
             network issue."
        );
    }
    println!("aozora-bump: Cargo.lock refreshed against {new_version}");
    Ok(())
}

/// Whether `s` is exactly three dot-separated runs of ASCII digits.
fn is_semver_triple(s: &str) -> bool {
    let mut parts = s.split('.');
    let triple = [parts.next(), parts.next(), parts.next()];
    parts.next().is_none()
        && triple.iter().all(|part| {
            part.is_some_and(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
        })
}

#[cfg(test)]
mod tests {
    use super::{
        RETIRED_UPSTREAM_PATHS, SCANNED_FILES, aozora_pin_pattern, fold_separators,
        is_semver_triple, scan_comments,
    };

    /// The comment marker a scanned file kind uses.
    fn marker(kind: &str) -> &'static str {
        SCANNED_FILES
            .iter()
            .find(|&&(k, _)| k == kind)
            .map(|&(_, m)| m)
            .expect("the kind is scanned")
    }

    #[test]
    fn semver_triple_accepts_a_full_version_and_rejects_the_rest() {
        assert!(is_semver_triple("0.5.0"));
        assert!(is_semver_triple("10.20.30"));
        assert!(!is_semver_triple("0.5"));
        assert!(!is_semver_triple("0.5.0.1"));
        assert!(!is_semver_triple("0.5.x"));
        assert!(!is_semver_triple("0..0"));
        assert!(!is_semver_triple(""));
    }

    /// The bump rewrites both manifest spellings — the workspace's caret
    /// requirement and the fuzz crate's exact one — and preserves the `=`.
    #[test]
    fn pin_pattern_rewrites_both_manifest_spellings() {
        let pattern = aozora_pin_pattern();
        let workspace = r#"aozora = { version = "0.4.1", default-features = false }"#;
        let fuzz =
            r#"aozora                    = { version = "=0.4.1", default-features = false }"#;
        assert_eq!(
            pattern.replace_all(workspace, "${1}0.5.0${3}"),
            r#"aozora = { version = "0.5.0", default-features = false }"#
        );
        assert_eq!(
            pattern.replace_all(fuzz, "${1}0.5.0${3}"),
            r#"aozora                    = { version = "=0.5.0", default-features = false }"#
        );
    }

    /// A sibling crate whose name merely starts with `aozora` must not be
    /// rewritten — the pattern anchors on the umbrella crate's own line.
    #[test]
    fn pin_pattern_leaves_sibling_crates_alone() {
        let pattern = aozora_pin_pattern();
        let sibling = "aozora-flavored-markdown = { version = \"0.4.1\", path = \"crates/aozora-flavored-markdown\" }";
        assert_eq!(pattern.find(sibling), None);
    }

    /// A banned entry that is spelled with a `-`, i.e. one whose intra-doc
    /// spelling differs from its manifest spelling.
    fn hyphenated_entry() -> &'static str {
        RETIRED_UPSTREAM_PATHS
            .iter()
            .map(|&(needle, _)| needle)
            .find(|needle| needle.contains('-'))
            .expect("the banned list names at least one retired crate")
    }

    #[test]
    fn flags_a_retired_path_in_an_inner_doc_comment() {
        // Assembled at runtime so this test's own source stays clean.
        let src = format!("//! Layers {} onto comrak.\n", RETIRED_UPSTREAM_PATHS[0].0);
        let hits = scan_comments(&src, marker("rs"));
        assert_eq!(hits.len(), 1, "expected one violation, got {hits:?}");
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].needle, RETIRED_UPSTREAM_PATHS[0].0);
    }

    #[test]
    fn flags_a_retired_path_in_an_outer_doc_comment() {
        let needle = hyphenated_entry();
        let src = format!("    /// Delegates to {needle}.\n");
        let hits = scan_comments(&src, marker("rs"));
        assert_eq!(hits.len(), 1, "expected one violation, got {hits:?}");
        assert_eq!(hits[0].needle, needle);
    }

    #[test]
    fn flags_the_underscore_spelling_of_a_retired_crate() {
        // The banned list is written in manifest spelling; an intra-doc link
        // uses the underscored one. Both must fail, or rustdoc references
        // outlive the crate they point at.
        let hyphenated = hyphenated_entry();
        let src = format!("/// Draws from [`{}`].\n", hyphenated.replace('-', "_"));
        let hits = scan_comments(&src, marker("rs"));
        assert_eq!(hits.len(), 1, "expected one violation, got {hits:?}");
        assert_eq!(hits[0].needle, hyphenated);
    }

    #[test]
    fn flags_a_retired_path_in_a_plain_comment() {
        // Prose rots whichever marker introduces it.
        let src = format!("    // Mirrors {}.\n", RETIRED_UPSTREAM_PATHS[0].0);
        let hits = scan_comments(&src, marker("rs"));
        assert_eq!(hits.len(), 1, "expected one violation, got {hits:?}");
        assert_eq!(hits[0].needle, RETIRED_UPSTREAM_PATHS[0].0);
    }

    #[test]
    fn flags_a_retired_crate_in_a_manifest_comment() {
        // A manifest note explaining why a lint is set the way it is names
        // upstream just as a doc comment does, and rots the same way. It went
        // uncaught while the gate read only `.rs`.
        let needle = hyphenated_entry();
        let src = format!("# Kept stricter than aozora: {needle} needs the allow.\n");
        let hits = scan_comments(&src, marker("toml"));
        assert_eq!(hits.len(), 1, "expected one violation, got {hits:?}");
        assert_eq!(hits[0].needle, needle);
    }

    #[test]
    fn a_manifest_key_is_not_a_manifest_comment() {
        // TOML has no line marker on real content, so only a leading `#`
        // counts — a dependency actually named after a retired crate would be
        // the compiler's problem, not this gate's.
        let needle = hyphenated_entry();
        let src = format!("{needle} = {{ version = \"0.4.1\" }}\n");
        assert!(scan_comments(&src, marker("toml")).is_empty());
    }

    #[test]
    fn ignores_code() {
        // Real code is out of scope: an import that no longer resolves is the
        // compiler's job, and this gate covers what the compiler cannot see.
        let needle = RETIRED_UPSTREAM_PATHS[0].0;
        let src = format!("use {needle}::thing;\nlet s = \"{needle}\";\n");
        assert!(scan_comments(&src, marker("rs")).is_empty());
    }

    #[test]
    fn clean_comments_produce_no_violations() {
        let src = "//! Layers the sibling parser onto comrak.\n/// Renders to HTML.\n";
        assert!(scan_comments(src, marker("rs")).is_empty());
    }

    #[test]
    fn reports_every_offending_line() {
        let needle = RETIRED_UPSTREAM_PATHS[0].0;
        let src = format!("//! {needle}\nlet filler = 1;\n/// {needle}\n");
        let hits = scan_comments(&src, marker("rs"));
        assert_eq!(hits.len(), 2);
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[1].line, 3);
    }

    #[test]
    fn banned_list_has_no_duplicates() {
        // Compared after folding: `a-b` and `a_b` are one entry to the gate,
        // so listing both would be a silent duplicate.
        let mut names: Vec<String> = RETIRED_UPSTREAM_PATHS
            .iter()
            .map(|&(n, _)| fold_separators(n))
            .collect();
        names.sort_unstable();
        let before = names.len();
        names.dedup();
        assert_eq!(before, names.len(), "duplicate entry in the banned list");
    }
}
