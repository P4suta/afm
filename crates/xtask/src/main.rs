//! Workspace automation: every task the `Justfile` or CI invokes that is not
//! a direct cargo call. `--help` lists the sub-commands.
//!
//! Aozora parser / corpus concerns live in the sibling `P4suta/aozora` repo
//! (ADR-0010), along with their refresh sub-commands.

#![forbid(unsafe_code)]

use core::cmp;
use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};

mod spec_refresh;

#[derive(Parser, Debug)]
#[command(version, about = "aozora-flavored-markdown workspace automation", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Fail if any line names a repo path that no longer exists, or if doc
    /// comments outgrow their pinned line budget.
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

/// One line naming a repo path that no longer exists.
#[derive(Debug, PartialEq, Eq)]
struct Violation {
    line: usize,
    needle: &'static str,
    why: &'static str,
    text: String,
}

/// Directories that are never authored here.
///
/// `target/` is cargo output (including each fuzz crate's own), `pkg/` is
/// wasm-pack output, `node_modules/` is bun's.
// The rest are generated or fetched too, and they matter because the scan
// below reads every file rather than a list of kinds: `dist/` is the release
// bundle xtask itself writes, `coverage/` an llvm-cov report, `corpus/` and
// `artifacts/` are cargo-fuzz's, `.cargo/` the registry cache `.gitignore`
// reserves at this path, `.vite/` the playground's cache.
const UNSCANNED_DIRS: &[&str] = &[
    "target",
    "pkg",
    "node_modules",
    ".git",
    "dist",
    "coverage",
    "corpus",
    "artifacts",
    ".cargo",
    ".vite",
];

// Repo-relative paths that no longer exist, each with the reason it is gone.
//
// This is what stayed behind when the retired *upstream* names moved to Vale
// (`styles/Aozora/RetiredPaths.yml`, DEV-221), and the reason it could not go
// with them is the reason it was written separately in the first place. A dead
// path rots in file content, not only in prose — a `linguist-vendored` glob, a
// CODEOWNERS owner line, a bacon watch list, a CI paths-filter — i.e. in files
// that carry no comment marker, that no compiler opens, and that a prose
// linter has no business reading. Deleting the vendored comrak tree (ADR-0024)
// left 15 files naming it, in four file kinds; exactly one of those hits was
// on a comment line. A hand sweep found the other 14.
//
// A path is a fact on disk, so the check needs no judgement: if the directory
// is gone, every line still naming it is wrong. `every_banned_repo_path_is_gone`
// holds the list to that fact.
const RETIRED_REPO_PATHS: &[(&str, &str)] = &[(
    "upstream/",
    "the vendored comrak tree is gone — comrak resolves from crates.io (ADR-0024)",
)];

// Where a retired path is a record rather than drift. The changelog says what
// was removed and a decision record says why; both are dated documents, and
// rewriting them to keep a lint quiet would delete the only account of the
// decision. A superseded decision is amended by reference — its Context keeps
// the world it was taken in, and its Decision says which ADR replaced it.
const HISTORY_PATHS: &[&str] = &["CHANGELOG.md", "docs/adr"];

// The file that defines `RETIRED_REPO_PATHS` necessarily spells every path it
// bans, so the scan skips it. `file!()` is the workspace-relative path cargo
// compiled this file from, so the exclusion cannot drift away from the list it
// exists for. (Vale solves the same problem for its own rule file the same
// way, in `.vale.ini`'s `[styles/**]` section.)
const RETIRED_PATH_LIST_FILE: &str = file!();

// Return every line in `src` that names a retired repo path.
//
// Whole lines, not just comments: `upstream/comrak/** linguist-vendored` is not
// a comment in any language, and it was exactly as wrong as the prose beside it.
fn scan_repo_paths(src: &str) -> Vec<Violation> {
    let mut out = Vec::new();
    for (idx, raw) in src.lines().enumerate() {
        for &(needle, why) in RETIRED_REPO_PATHS {
            if raw.contains(needle) {
                out.push(Violation {
                    line: idx + 1,
                    needle,
                    why,
                    // A generated bundle is one enormous line; report enough to
                    // find it and no more.
                    text: raw.trim().chars().take(160).collect(),
                });
            }
        }
    }
    out
}

/// Every file under `dir`, skipping the directories nobody here authors and
/// the documents where a retired path is a record.
fn collect_scannable_files(dir: &Path, root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading an entry of {}", dir.display()))?;
        let path = entry.path();
        let name = entry.file_name();
        if path.is_dir() {
            if name.to_str().is_some_and(|n| UNSCANNED_DIRS.contains(&n)) {
                continue;
            }
            collect_scannable_files(&path, root, out)?;
            continue;
        }
        let relative = path.strip_prefix(root).unwrap_or(path.as_path());
        if HISTORY_PATHS.iter().any(|h| relative.starts_with(h)) {
            continue;
        }
        out.push(path);
    }
    out.sort();
    Ok(())
}

/// Fail when a line under `root` names a repo path that no longer exists, or
/// when doc comments have outgrown [`MAX_DOC_LINES`].
fn comment_discipline(root: &Path) -> Result<()> {
    if !root.is_dir() {
        bail!(
            "comment-discipline: {} not found; run from the workspace root",
            root.display()
        );
    }

    let mut files = Vec::new();
    collect_scannable_files(root, root, &mut files)?;

    let mut total = 0usize;
    for path in &files {
        if path.ends_with(RETIRED_PATH_LIST_FILE) {
            continue;
        }
        let src = match fs::read_to_string(path) {
            Ok(src) => src,
            // Not UTF-8, so not text to scan: a fuzz corpus seed, a wasm
            // bundle. Any other failure is real — the walk just listed this
            // path, so not being able to open it is the error it looks like.
            Err(e) if e.kind() == io::ErrorKind::InvalidData => continue,
            Err(e) => return Err(e).with_context(|| format!("reading {}", path.display())),
        };

        for v in scan_repo_paths(&src) {
            if total == 0 {
                println!("comment-discipline: lines naming something retired:");
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
            "comment-discipline: {total} reference(s) to a repo path that no longer exists. \
             There is nothing to describe instead: the file is gone, so the line is wrong. \
             (A retired *upstream* name in prose is `just vale`'s finding, not this one.)"
        );
    }

    println!(
        "comment-discipline: clean ({} file(s), {} repo path(s) checked)",
        files.len(),
        RETIRED_REPO_PATHS.len()
    );

    doc_volume_ratchet(root)
}

// ---------------------------------------------------------------------------
// doc-comment volume ratchet
// ---------------------------------------------------------------------------

/// Ceiling on doc-comment lines, pinned to today's count.
///
/// The gate is an absolute count and not a share of source, because a share
/// has a denominator. Pinned to today, `doc / all` fails any commit that nets
/// even one fewer *non-doc* line — a plain `refactor:` — and the only way out
/// of that failure is to raise the ceiling. A ratchet that ordinary work can
/// force upward is not a ratchet. An absolute count moves only when prose
/// moves, which is the thing being held.
///
/// **It only ever moves down.** Nothing computes or rewrites it: lowering it
/// after a cut is bookkeeping, raising it is a hand edit that shows up in
/// review as exactly what it is — a decision to let prose grow.
///
/// The failure is never "delete a comment". It is: say *why*, once, in the
/// place a reader will meet the constraint — and stop restating what the
/// types and the code already say.
// Lowered by 38 when the vendored-comrak machinery left this file
// (ADR-0024): `upstream_diff`, `upstream_sync` and `copy_dir_recursive`, plus
// their sub-command docs, took their prose with them. Bookkeeping after a cut,
// not a decision.
//
// Raised by 6 for six new public items, one line each, none of them a
// restatement of a signature: `Span::new` / `Span::len` / `Span::is_empty`,
// `From<Span> for Range<usize>`, `Position::new` and `Range::new`. Each line
// says the one thing the type does not — that the pair is unordered, that
// `len` saturates on a reversed span, that the empty span is the shape a
// document-scoped diagnostic carries, that the `From` is for slicing the
// source, and that the coordinates are 1-based.
//
// Lowered by 22 when comrak left the public surface: the `Options` escape
// hatches and their security warnings, the two getters and the raw-HTML
// constructors' prose all went, and the typed `with_*` builders that replaced
// them each say their one thing in a line. Bookkeeping after a cut.
//
// Raised by 13 for the crate's first public error type. Nine are `Error`
// itself: which of the two failures a caller can provoke, and — the
// load-bearing line — that the rendering entry points deliberately do *not*
// gain a `Result`, because CommonMark is a total grammar and a diagnostic has
// a warning's standing. Four are `canonicalize`'s `# Errors` section, which
// `clippy::missing_errors_doc` requires of a public `Result`, and the `?` its
// doctest now needs. None of it restates the signature: the signature says
// `Result<String, Error>` and stops there.
//
// Lowered by 4 when the `html` module was flattened into the root. Its 17
// lines went; 13 came back, and none of them restate a signature: 10 for
// `to_html`, which has to say what it drops and where to go instead; 3 for
// `RenderedBlocks`, two of them the line saying why diagnostics are
// document-scoped — moved off `render_blocks`, which is 2 shorter for it —
// and 2 in `classes`, where `is_known` now owns the numeric-family rule and
// has to state it. Bookkeeping after a cut, not a decision.
//
// Raised by 151 — the largest single raise this constant will take, and a
// decision, not bookkeeping. `missing_docs` is now `warn` in
// `[workspace.lints.rust]`, which `just clippy`'s `-D warnings` makes a gate,
// and it found 145 public items carrying no doc comment at all: every
// `ir::Block` / `ir::Inline` variant and field, `Document` / `ListItem` /
// `TableRow` / `TableAlign`, the `Range` + `Position` leaves, `RenderedIr` /
// `RenderedBlock` / `RenderedBlocks` and `Error::SourceTooLarge`'s `len`, the
// whole epub `Error` tree, test-support's `Violation` / `WellFormedError`, and
// the two wasm envelopes. 151 lines for 145 items: six needed a second line.
//
// None of it is restatement, because there was nothing there to restate. Each
// line says what the type cannot: that `source_line` is `None` below top
// level, that `range` is `None` when the parser reported the position
// inverted, that `TableAlign::Default` is a value and not a missing marker,
// that `Image`'s `alt` is still inlines. The alternative — paying for a
// mandatory-prose lint by cutting explanation somewhere else — would trade
// documentation a reader asked for against documentation a reader needed, so
// the raise is the honest move and it is recorded here as one.
//
// Lowered by 2 in the same change: the test module's `SOURCE_LINES` — a
// hand-written snapshot of the workspace size, 2 677 lines stale — became a
// live measurement, and its doc comment went with it. Bookkeeping after a cut.
// It is written down because it now has to be: the ceiling is asserted equal
// to the measured count (`the_pinned_ceiling_is_the_count_it_is_pinned_to`),
// so silent headroom is no longer somewhere prose can accumulate.
//
// Raised by 11 for one new public item, `diagnose`. It is the entry point the
// `check` sub-command needed and the crate did not have: `check` documented
// itself as parsing "without rendering" and called `render` anyway, because
// nothing else could answer what the lexer saw. Five lines are the contract a
// reader needs — that these are `render`'s diagnostics exactly, so the two can
// never disagree about a source, and that what is skipped is the comrak parse,
// the splice and the formatting. Six are the doctest, which is what holds the
// first half of that claim. None of it restates the signature, which says
// `Vec<Diagnostic>` and stops there. The two CLIs moved out of their `main.rs`
// files in the same change and carry their prose with them, so the split
// itself costs nothing here.
//
// Lowered by 40 when the retired-upstream-path scan left this file for Vale
// (DEV-221). Thirty-seven are the prose that went with the machinery —
// `RETIRED_UPSTREAM_PATHS` and its rationale, `fold_separators`,
// `SCANNED_FILES` and `scan_comments` — and three are the two test helpers
// that read the banned list. None of it was deleted: the same explanations
// are in `styles/Aozora/RetiredPaths.yml` and `.vale.ini`, beside the rule
// they now describe. Bookkeeping after a cut, not a decision.
//
// Re-pinned from 1 699 to 5 191 because the POPULATION grew, not the budget.
// `collect_crate_sources` now walks every directory `DOC_RATCHET_DIRECTORIES`
// names, where it walked only `src`. This is not a raise and buys no slack:
// it is the same zero headroom over a set that is 3 492 doc lines larger, all
// of which were already written and none of which any gate could see.
//
// The ratchet existed to stop rustdoc restatement accumulating, and its walk
// did not contain the place it accumulated. `crates/*/tests` carried 3 424 doc
// lines against the 1 699 governed here — twice the budget, ungoverned — and
// `gate_wiring.rs` alone carried 1 201 of them, a single file within 500 lines
// of the entire watched surface. So one `///` under `src/` was refused
// outright while several hundred added to a test file cost nothing, which is
// the gradient that built the machinery this change is cutting back.
//
// What it still does not measure, beyond the directories left out below:
// plain `//` notes anywhere. That is the live hole — `gate_wiring.rs` carries
// 1 634 plain-comment lines against its 1 201 rustdoc ones, so prose demoted
// from `///` to `//` is free, and this comment is itself spending it.
//
// Lowered by 1 in the same change: `DOC_RATCHET_ROOT`'s own doc comment lost
// the sentence carving `tests/` out, there being nothing left to carve.
// Lowered by 34 more: the `REPOSITORY_SETTINGS` family deleted from
// `gate_wiring.rs` took its prose with it — the first deletion this ceiling
// has ever been able to charge for, and the reason to widen it here rather
// than file it.
// Lowered by 4 more: `gate_wiring.rs`'s `PUBLICATION_ADR` block lost the two
// `///` pairs that restated a constant's own value and a function's own name.
// Lowered by 2 more: `vale_or_the_bridge_that_installs_it` lost the half of
// its doc that described the Dockerfile assertion deleted from its body, the
// version pin now being compiled rather than asserted to contain a substring.
// Re-measured, not estimated — the slack a deletion buys is spent by lowering
// this, or it gets spent by the next `///` nobody argued for.
const MAX_DOC_LINES: u64 = 5_185;

/// Backstop on doc lines as a share of source, in parts per 100 000, held at
/// the sibling `aozora` crate's own ~16.5% rather than at today's measured
/// share. Slack is the point: it catches the one case [`MAX_DOC_LINES`]
/// cannot — source shrinking out from under a doc budget that was
/// proportionate at the old size — without firing on a refactor that merely
/// deletes some code.
const MAX_DOC_RATIO_PER_100K: u64 = 16_500;

/// Where the ratchet measures: every crate under this directory, read in the
/// target directories `DOC_RATCHET_DIRECTORIES` names. Nothing carves test
/// code out of the count, so the share reported below is of all source, not
/// of production source alone.
const DOC_RATCHET_ROOT: &str = "crates";

// Cargo's own target directories for hand-written code, less `examples/` (an
// example is documentation by nature) and the nested `fuzz/` workspace, which
// is not a target of any crate here. Named once because the walk and all four
// failure messages below need the same list: spelled out separately in each,
// they said `src/` and `tests/` for as long as the walk had already been
// reading `benches/` too — the drift this file's own gate exists to refuse.
const DOC_RATCHET_DIRECTORIES: [&str; 3] = ["src", "tests", "benches"];

/// A doc line is one whose first non-space token opens a rustdoc comment.
/// Plain `//` notes are not counted — they cost a reader nothing until they
/// go stale, which the retired-path gate above already covers.
fn count_doc_lines(src: &str) -> (u64, u64) {
    let mut doc = 0u64;
    let mut all = 0u64;
    for line in src.lines() {
        all += 1;
        let trimmed = line.trim_start();
        if trimmed.starts_with("///") || trimmed.starts_with("//!") {
            doc += 1;
        }
    }
    (doc, all)
}

/// Why a measured `(doc, all)` fails, or `None` when it clears both gates.
///
/// Factored out of the walk so the tests can pin the asymmetry the gates
/// exist for: prose growing must fail, code shrinking must not. The ratio is
/// compared by cross-multiplication, so the verdict cannot drift with float
/// rounding.
fn doc_budget_failure(doc: u64, all: u64) -> Option<String> {
    if doc > MAX_DOC_LINES {
        return Some(format!(
            "comment-discipline: {doc} doc-comment lines under {DOC_RATCHET_ROOT}/*/{{{}}}, over \
             the pinned ceiling of {MAX_DOC_LINES}. Cut restatements of what the code already \
             says, keep the \"why\". Raising MAX_DOC_LINES is a deliberate hand edit, not a fix.",
            DOC_RATCHET_DIRECTORIES.join(",")
        ));
    }
    if doc * 100_000 > all * MAX_DOC_RATIO_PER_100K {
        let measured = doc * 100_000 / all;
        return Some(format!(
            "comment-discipline: doc comments are {}.{:03}% of source ({doc}/{all}), over the \
             {}.{:03}% backstop. The source shrank far enough that the surviving prose is now out \
             of proportion to it; cut prose to match.",
            measured / 1000,
            measured % 1000,
            MAX_DOC_RATIO_PER_100K / 1000,
            MAX_DOC_RATIO_PER_100K % 1000,
        ));
    }
    None
}

/// Every `.rs` file a crate keeps in a `DOC_RATCHET_DIRECTORIES` directory.
fn collect_crate_sources(root: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let crates = root.join(DOC_RATCHET_ROOT);
    let entries = fs::read_dir(&crates).with_context(|| format!("reading {}", crates.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading an entry of {}", crates.display()))?;
        // A budget that stopped at `src/` was one the prose walked around: the
        // integration tests carried twice the rustdoc the ceiling governed, so
        // a `///` cost nothing there and was refused outright one directory
        // over. Walking Cargo's whole set rather than the two directories that
        // happen to be heavy today is what keeps `benches/` from being the
        // next place it accumulates.
        for directory in DOC_RATCHET_DIRECTORIES {
            let measured = entry.path().join(directory);
            if measured.is_dir() {
                collect_rust_sources(&measured, out)?;
            }
        }
    }
    out.sort();
    Ok(())
}

fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let entries = fs::read_dir(dir).with_context(|| format!("reading {}", dir.display()))?;
    for entry in entries {
        let entry = entry.with_context(|| format!("reading an entry of {}", dir.display()))?;
        let path = entry.path();
        if path.is_dir() {
            collect_rust_sources(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
            out.push(path);
        }
    }
    Ok(())
}

/// Fail when doc comments have outgrown [`MAX_DOC_LINES`], or the source has
/// shrunk far enough past them to breach [`MAX_DOC_RATIO_PER_100K`].
fn doc_volume_ratchet(root: &Path) -> Result<()> {
    let mut files = Vec::new();
    collect_crate_sources(root, &mut files)?;

    let mut doc = 0u64;
    let mut all = 0u64;
    let mut worst: Vec<(u64, u64, PathBuf)> = Vec::new();
    for path in files {
        let src =
            fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let (file_doc, file_all) = count_doc_lines(&src);
        doc += file_doc;
        all += file_all;
        worst.push((file_doc, file_all, path));
    }

    if all == 0 {
        bail!("comment-discipline: no crate source found under {DOC_RATCHET_ROOT}/");
    }

    if let Some(reason) = doc_budget_failure(doc, all) {
        worst.sort_by_key(|&(file_doc, _, _)| cmp::Reverse(file_doc));
        println!("comment-discipline: heaviest files (doc lines / total):");
        for (file_doc, file_all, path) in worst.iter().take(5) {
            println!("  {file_doc:>5} / {file_all:<5}  {}", path.display());
        }
        bail!(reason);
    }

    let measured = doc * 100_000 / all;
    println!(
        "comment-discipline: {doc}/{MAX_DOC_LINES} doc lines, {}.{:03}% of {all} source lines \
         (backstop {}.{:03}%)",
        measured / 1000,
        measured % 1000,
        MAX_DOC_RATIO_PER_100K / 1000,
        MAX_DOC_RATIO_PER_100K % 1000,
    );
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

    // Refresh the workspace Cargo.lock. The cargo-fuzz crate's lock is NOT
    // refreshed here: it is a second workspace with a second committed
    // lockfile (DEV-293 — the sentence this comment used to carry, that the
    // file is git-ignored and regenerated on the next `cargo fuzz` build, has
    // been false since it was committed). Re-resolving it needs a cargo run
    // with a different `--manifest-path`, which is `just fuzz-lock`; until it
    // is run the two lockfiles disagree about `aozora` and both
    // `verify-version-pins` and `lock_binding.rs` say so.
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

// ---------------------------------------------------------------------------
// dependency provenance
// ---------------------------------------------------------------------------
//
// A test rather than a sub-command, deliberately. ADR-0024 retires the
// vendored tree on the argument that no replacement gate is needed — the
// `checksum` in `Cargo.lock` is verified by cargo on every build, which is
// strictly stronger than any grep. That is true exactly while the checksum is
// *there*, and re-introducing a `path` dependency silently removes it. So this
// asserts the premise of the argument, and adds no gate to argue with.
#[cfg(test)]
mod provenance {
    // The one source every crate this workspace does not own must resolve from.
    //
    // Vendoring made cargo resolve a dependency by `path` here while `cargo
    // publish` stripped that `path` and handed consumers the registry crate:
    // one manifest, two build graphs, and the local one carried no checksum for
    // anything to verify. `Cargo.lock` records which of the two is in force, so
    // the lockfile — not a policy sentence — is where "we build what we ship"
    // is decidable (ADR-0024).
    pub(super) const CRATES_IO_SOURCE: &str =
        "registry+https://github.com/rust-lang/crates.io-index";

    // A `[[package]]` block of `Cargo.lock`, reduced to its provenance. `source`
    // is absent exactly when cargo resolved the package from inside this repo.
    #[derive(Debug, Default, PartialEq, Eq)]
    pub(super) struct LockedPackage {
        pub(super) name: String,
        pub(super) source: Option<String>,
        pub(super) checksum: Option<String>,
    }

    // Read the `[[package]]` blocks of a lockfile.
    //
    // Hand-read rather than pulling a TOML parser into xtask for three keys:
    // `Cargo.lock` is machine-written in one shape — a `[[package]]` header,
    // then `key = "value"` lines — that cargo has kept stable across lock
    // formats. Any other table header ends the current block, so a trailing
    // `[metadata]` cannot have its keys read onto the last package.
    pub(super) fn parse_locked_packages(lock: &str) -> Vec<LockedPackage> {
        let mut out = Vec::new();
        let mut current: Option<LockedPackage> = None;
        for line in lock.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                out.extend(current.take());
                if line == "[[package]]" {
                    current = Some(LockedPackage::default());
                }
                continue;
            }
            let Some(pkg) = current.as_mut() else {
                continue;
            };
            if let Some(name) = quoted_value(line, "name") {
                pkg.name = name;
            } else if let Some(source) = quoted_value(line, "source") {
                pkg.source = Some(source);
            } else if let Some(checksum) = quoted_value(line, "checksum") {
                pkg.checksum = Some(checksum);
            }
        }
        out.extend(current);
        out
    }

    // `key = "value"` -> `value`, for that exact key. A longer key that merely
    // starts with it (`names = …`) fails at the `=`, and a `dependencies` list
    // entry has no `=` at all.
    fn quoted_value(line: &str, key: &str) -> Option<String> {
        let rest = line.strip_prefix(key)?.trim_start();
        let value = rest.strip_prefix('=')?.trim_start().strip_prefix('"')?;
        let end = value.find('"')?;
        Some(value[..end].to_owned())
    }

    // The package names cargo resolves from inside this repo. A `[[package]]`
    // with no `source` is legitimate for exactly these.
    pub(super) fn workspace_member_names(manifest: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut inside = false;
        for line in manifest.lines() {
            let line = line.trim();
            if !inside {
                inside = line.starts_with("members") && line.contains('[');
                continue;
            }
            if line.starts_with(']') {
                break;
            }
            if let Some(path) = line.split('"').nth(1) {
                out.push(path.rsplit('/').next().unwrap_or(path).to_owned());
            }
        }
        out
    }

    // Dependency lines in a manifest that carry a `path`, with line numbers.
    fn manifest_dependency_paths(manifest: &str) -> Vec<(usize, String)> {
        let pattern = regex::Regex::new(r#"^[A-Za-z0-9_-]+\s*=\s*\{[^}]*\bpath\s*=\s*"([^"]+)""#)
            .expect("dependency path pattern compiles");
        manifest
            .lines()
            .enumerate()
            .filter_map(|(idx, line)| pattern.captures(line).map(|c| (idx + 1, c[1].to_owned())))
            .collect()
    }

    // Every way the graph resolved here can stop being the graph a consumer
    // gets: a tree vendored into the repo, a source that is not the public
    // registry, or a lock entry without the checksum cargo re-verifies on
    // every build.
    pub(super) fn dependency_provenance_failures(manifest: &str, lock: &str) -> Vec<String> {
        let members = workspace_member_names(manifest);
        let mut out = Vec::new();

        for pkg in parse_locked_packages(lock) {
            if members.contains(&pkg.name) {
                continue;
            }
            let Some(source) = pkg.source.as_deref() else {
                out.push(format!(
                    "{}: locked with no `source`, i.e. resolved from a path inside this repo. \
                     `cargo publish` strips the path, so consumers would compile a graph nobody \
                     built here (ADR-0024).",
                    pkg.name
                ));
                continue;
            };
            if source != CRATES_IO_SOURCE {
                out.push(format!(
                    "{}: resolves from `{source}`, not crates.io.",
                    pkg.name
                ));
                continue;
            }
            if pkg.checksum.is_none() {
                out.push(format!(
                    "{}: locked without a `checksum`; nothing verifies the crate on build.",
                    pkg.name
                ));
            }
        }

        for (line, path) in manifest_dependency_paths(manifest) {
            if !path.starts_with("crates/") {
                out.push(format!(
                    "Cargo.toml:{line}: dependency path `{path}` leaves `crates/` — a tree \
                     vendored into this repo builds locally and vanishes on publish (ADR-0024)."
                ));
            }
        }

        out
    }
}

#[cfg(test)]
mod tests {
    use super::provenance::{
        CRATES_IO_SOURCE, dependency_provenance_failures, parse_locked_packages,
        workspace_member_names,
    };
    use super::{
        DOC_RATCHET_DIRECTORIES, DOC_RATCHET_ROOT, MAX_DOC_LINES, MAX_DOC_RATIO_PER_100K, Path,
        PathBuf, RETIRED_PATH_LIST_FILE, RETIRED_REPO_PATHS, aozora_pin_pattern,
        collect_crate_sources, count_doc_lines, doc_budget_failure, fs, is_semver_triple,
        scan_repo_paths,
    };

    // The `(doc, all)` the gate itself would measure — the same walk over the
    // same files, run here instead of read off a constant.
    //
    // It was a constant, `SOURCE_LINES`, whose own doc comment licensed it to
    // drift ("only its distance from the backstop's floor matters"). By the
    // time this was written it stood at 10 398 against a real 13 075: a number
    // nothing recomputed, standing in for the workspace in every assertion
    // below. The licence holds only while the distance stays positive, and it
    // is a `u64` subtraction — so past a high enough ceiling a stale snapshot
    // does not report a thin margin, it panics inside the test that exists to
    // show the gate has slack. Measuring costs one directory walk and cannot
    // go stale at all.
    fn measured_source() -> (u64, u64) {
        let mut files = Vec::new();
        collect_crate_sources(&repo_root(), &mut files).expect("walking the crate source trees");
        let mut doc = 0;
        let mut all = 0;
        for path in &files {
            let src = fs::read_to_string(path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            let (file_doc, file_all) = count_doc_lines(&src);
            doc += file_doc;
            all += file_all;
        }
        assert!(
            all > 1_000,
            "only {all} source line(s) found under {DOC_RATCHET_ROOT}/*/{{{}}}; the walk is \
             reading nothing",
            DOC_RATCHET_DIRECTORIES.join(",")
        );
        (doc, all)
    }

    #[test]
    fn doc_line_count_sees_both_rustdoc_markers_and_ignores_plain_notes() {
        let (doc, all) =
            count_doc_lines("//! module\n/// item\n    /// indented\n// plain note\nfn f() {}\n\n");
        assert_eq!((doc, all), (3, 6));
    }

    // The ceiling is the count, not a budget the count sits under.
    //
    // The gate fails on `doc > MAX_DOC_LINES`, so every doc line deleted
    // without the matching edit here leaves headroom — and headroom is prose
    // that can be added later without the hand edit that is this constant's
    // entire reason to exist. It is not hypothetical: the ceiling stood at
    // 1 580 against a measured 1 579, so one `///` line could have gone in
    // free, and nothing in the repo could have said so. `just
    // comment-discipline` cannot: a count under its ceiling is exactly what it
    // is built to pass.
    //
    // Failing here reads as "do the bookkeeping the constant's own doc comment
    // describes": below, lower it and say what was cut; above, this is the
    // deliberate raise, so raise it and say what grew.
    #[test]
    fn the_pinned_ceiling_is_the_count_it_is_pinned_to() {
        let (doc, _) = measured_source();
        assert_eq!(
            doc,
            MAX_DOC_LINES,
            "MAX_DOC_LINES is {MAX_DOC_LINES} and {DOC_RATCHET_ROOT}/*/{{{}}} carries {doc} doc \
             line(s). The ceiling is pinned to today's count — a gap either way is a hand edit \
             that was not made.",
            DOC_RATCHET_DIRECTORIES.join(",")
        );
    }

    /// The ratchet's whole point: one doc line more than the pinned count
    /// fails, however the surrounding source moved.
    #[test]
    fn one_extra_doc_line_breaks_the_pinned_ceiling() {
        let (_, all) = measured_source();
        assert!(
            doc_budget_failure(MAX_DOC_LINES + 1, all + 1).is_some(),
            "a doc line added along with its file must not fit"
        );
        assert!(
            doc_budget_failure(MAX_DOC_LINES + 1, all).is_some(),
            "a plain note promoted to a doc comment must not fit either"
        );
    }

    /// The regression an earlier draft shipped: pinning the *ratio* to today
    /// made a `refactor:` that deleted code — and no prose at all — fail, and
    /// the only way out of that failure was to raise the ceiling. Deleting
    /// source must stay green, or the ratchet is one ordinary commit can
    /// force upward.
    #[test]
    fn deleting_source_lines_does_not_trip_the_ratchet() {
        // How far the source may shrink at the pinned doc count before the
        // backstop fires. A ratio pinned to today's measurement left none.
        let (_, all) = measured_source();
        let floor = MAX_DOC_LINES * 100_000 / MAX_DOC_RATIO_PER_100K;
        let slack = all.saturating_sub(floor);
        assert!(
            slack >= 500,
            "only {slack} non-doc line(s) of slack: {all} source lines against a backstop floor \
             of {floor} at a ceiling of {MAX_DOC_LINES}. Raising the ceiling raises that floor, \
             so the backstop is now close enough to fire on a plain deletion."
        );

        assert!(doc_budget_failure(MAX_DOC_LINES, all).is_none());
        for shrunk_by in [1, 100, 500] {
            assert!(
                doc_budget_failure(MAX_DOC_LINES, all - shrunk_by).is_none(),
                "removing {shrunk_by} non-doc line(s) must not fail a doc-comment gate"
            );
        }
    }

    /// Deleting prose always passes, so the gate never blocks the fix it asks
    /// for.
    #[test]
    fn removing_a_doc_line_stays_within_budget() {
        let (_, all) = measured_source();
        assert!(doc_budget_failure(MAX_DOC_LINES - 1, all).is_none());
        assert!(doc_budget_failure(0, 1).is_none());
    }

    /// What the backstop is for: source shrinking far enough that the prose
    /// that survived is out of proportion to it.
    #[test]
    fn the_ratio_backstop_catches_source_collapsing_under_the_prose() {
        assert!(doc_budget_failure(MAX_DOC_LINES, 1_000).is_some());
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

    // --- retired repo paths -------------------------------------------------
    //
    // What no prose linter can see. When the vendored comrak tree was deleted,
    // 15 files still named it — a `linguist-vendored` glob, a CODEOWNERS
    // entry, a bacon watch list, a CI paths-filter, a PR-template line, a
    // coverage-ignore regex, a typos exclusion — and almost none of those
    // lines is a comment, or even prose. Replaying the pre-change tree through
    // `scan_repo_paths` flags all 15.

    // The banned path, assembled so this file's own prose stays clean whatever
    // the scan skips.
    fn retired_repo_path() -> &'static str {
        RETIRED_REPO_PATHS
            .first()
            .map(|&(path, _)| path)
            .expect("the banned list names at least one retired repo path")
    }

    #[test]
    fn flags_a_retired_repo_path_outside_any_comment() {
        let src = format!("{}comrak/** linguist-vendored\n", retired_repo_path());
        let hits = scan_repo_paths(&src);
        assert_eq!(hits.len(), 1, "expected one violation, got {hits:?}");
        assert_eq!(hits[0].line, 1);
        assert_eq!(hits[0].needle, retired_repo_path());
    }

    #[test]
    fn flags_a_retired_repo_path_in_a_file_kind_with_no_comment_marker() {
        // A CI paths-filter and a CODEOWNERS line, in files the comment scan
        // never opened and could not have parsed if it had.
        let src = format!(
            "      - '{path}**'\n/{path}comrak/ @P4suta\n",
            path = retired_repo_path()
        );
        let hits = scan_repo_paths(&src);
        assert_eq!(hits.len(), 2, "expected two violations, got {hits:?}");
        assert_eq!((hits[0].line, hits[1].line), (1, 2));
    }

    #[test]
    fn a_live_path_is_not_a_retired_one() {
        assert!(scan_repo_paths("watch = [\"crates/xtask/src\"]\n").is_empty());
    }

    #[test]
    fn a_very_long_line_is_reported_in_full_only_as_far_as_it_is_useful() {
        // Generated bundles are one line of megabytes; the report has to stay
        // readable or the gate is one nobody runs twice.
        let src = format!("{}{}", retired_repo_path(), "x".repeat(10_000));
        let hits = scan_repo_paths(&src);
        assert_eq!(hits.len(), 1);
        assert!(hits[0].text.chars().count() <= 160);
    }

    // The list is a claim about the filesystem, so the filesystem settles it.
    // A path that came back for a good reason must fail here — loudly, in the
    // gate's own tests — rather than fail every unrelated PR from then on.
    #[test]
    fn every_banned_repo_path_is_gone() {
        let root = repo_root();
        for &(path, why) in RETIRED_REPO_PATHS {
            let full = root.join(path);
            assert!(
                !full.exists(),
                "{} exists again, but the ban says: {why}",
                full.display()
            );
        }
    }

    // The path scan skips the one file that must spell every path it bans. If
    // `file!()` ever stopped naming that file the exclusion would silently
    // cover nothing — or, worse, cover some other file.
    #[test]
    fn the_skipped_file_is_the_one_that_defines_the_ban_list() {
        let listed = repo_root().join(RETIRED_PATH_LIST_FILE);
        assert!(listed.is_file(), "{} is not a file", listed.display());
        let src = fs::read_to_string(&listed).expect("reading the ban-list file");
        assert!(
            src.contains("const RETIRED_REPO_PATHS"),
            "{} does not define the ban list",
            listed.display()
        );
    }

    // --- dependency provenance ----------------------------------------------

    // The workspace root, from the manifest dir cargo compiled this crate in.
    fn repo_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
    }

    // A minimal manifest declaring this workspace's members.
    fn members_manifest() -> &'static str {
        "[workspace]\nmembers = [\n    \"crates/xtask\",\n]\n"
    }

    // The exact provenance the vendored tree had: resolved by `path`, so the
    // lockfile recorded neither a source nor a checksum for it, while `cargo
    // publish` stripped the path and handed consumers registry comrak. Four
    // gates believed they were holding the two together; the one that named
    // itself for the job grepped a markdown file for the words "0-line" and
    // computed no diff at all, so a hand edit sat inside the "verbatim" tree
    // for months (ADR-0024).
    #[test]
    fn a_path_resolved_dependency_has_no_provenance() {
        let lock = "[[package]]\nname = \"comrak\"\nversion = \"0.52.0\"\ndependencies = [\n \"entities\",\n]\n";
        let failures = dependency_provenance_failures(members_manifest(), lock);
        assert_eq!(failures.len(), 1, "expected one failure, got {failures:?}");
        assert!(failures[0].contains("comrak"), "{failures:?}");
        assert!(failures[0].contains("no `source`"), "{failures:?}");
    }

    // The same divergence written the other way round — in the manifest, where
    // a reviewer would actually meet it.
    #[test]
    fn a_dependency_path_leaving_crates_has_no_provenance() {
        let manifest = format!(
            "{}\n[workspace.dependencies]\ncomrak = {{ version = \"0.52.0\", path = \"{}comrak\" }}\n",
            members_manifest(),
            retired_repo_path()
        );
        let failures = dependency_provenance_failures(&manifest, "");
        assert_eq!(failures.len(), 1, "expected one failure, got {failures:?}");
        assert!(failures[0].contains("leaves `crates/`"), "{failures:?}");
    }

    #[test]
    fn a_dependency_path_inside_crates_is_fine() {
        let manifest = format!(
            "{}\n[workspace.dependencies]\naozora-flavored-markdown-test-support = {{ path = \"crates/aozora-flavored-markdown-test-support\" }}\n",
            members_manifest()
        );
        assert!(dependency_provenance_failures(&manifest, "").is_empty());
    }

    #[test]
    fn a_git_source_has_no_provenance() {
        // The pre-ADR-0015 shape for the sibling parser. A git rev is not on
        // crates.io either, so it is the same divergence as a path.
        let lock = "[[package]]\nname = \"aozora\"\nversion = \"0.5.0\"\nsource = \"git+https://github.com/P4suta/aozora.git#a53c632\"\n";
        let failures = dependency_provenance_failures(members_manifest(), lock);
        assert_eq!(failures.len(), 1, "expected one failure, got {failures:?}");
        assert!(failures[0].contains("not crates.io"), "{failures:?}");
    }

    #[test]
    fn a_registry_dependency_without_a_checksum_has_no_provenance() {
        let lock = format!(
            "[[package]]\nname = \"comrak\"\nversion = \"0.52.0\"\nsource = \"{CRATES_IO_SOURCE}\"\n"
        );
        let failures = dependency_provenance_failures(members_manifest(), &lock);
        assert_eq!(failures.len(), 1, "expected one failure, got {failures:?}");
        assert!(failures[0].contains("checksum"), "{failures:?}");
    }

    #[test]
    fn a_workspace_member_may_resolve_from_inside_the_repo() {
        let lock = "[[package]]\nname = \"xtask\"\nversion = \"0.5.0\"\n";
        assert!(dependency_provenance_failures(members_manifest(), lock).is_empty());
    }

    #[test]
    fn a_trailing_table_does_not_inherit_the_last_package() {
        // `[metadata]` after the packages must not have its keys read onto the
        // last `[[package]]`, or a source could be invented for one.
        let lock = format!(
            "[[package]]\nname = \"comrak\"\n[metadata]\nsource = \"{CRATES_IO_SOURCE}\"\nchecksum = \"aac0\"\n"
        );
        let packages = parse_locked_packages(&lock);
        assert_eq!(packages.len(), 1);
        assert_eq!(packages[0].source, None);
    }

    #[test]
    fn member_names_are_read_from_the_manifest_paths() {
        let manifest = "[workspace]\nresolver = \"3\"\nmembers = [\n    \"crates/one\",\n    # a note\n    \"crates/two\",\n]\nexclude = [\"crates/three\"]\n";
        assert_eq!(workspace_member_names(manifest), ["one", "two"]);
    }

    // The acceptance criterion this change was written against, made
    // executable — and widened from "comrak's lock entry has a source and a
    // checksum" to every dependency, because "comrak specifically" is what the
    // gates that missed the divergence already believed they were checking.
    #[test]
    fn every_dependency_in_the_workspace_lockfile_comes_from_the_registry() {
        let root = repo_root();
        let manifest =
            fs::read_to_string(root.join("Cargo.toml")).expect("reading the workspace manifest");
        let lock =
            fs::read_to_string(root.join("Cargo.lock")).expect("reading the workspace lockfile");

        // Guard the invariant against passing vacuously: a parser that read
        // nothing would satisfy every assertion below it.
        let packages = parse_locked_packages(&lock);
        assert!(
            packages.len() > 100,
            "only {} package(s) parsed out of Cargo.lock",
            packages.len()
        );
        let comrak = packages
            .iter()
            .find(|p| p.name == "comrak")
            .expect("comrak is in the lockfile");
        assert_eq!(comrak.source.as_deref(), Some(CRATES_IO_SOURCE));
        assert!(
            comrak.checksum.is_some(),
            "comrak is locked without a checksum"
        );
        assert_eq!(workspace_member_names(&manifest).len(), 7);

        let failures = dependency_provenance_failures(&manifest, &lock);
        assert!(
            failures.is_empty(),
            "dependency provenance:\n{}",
            failures.join("\n")
        );
    }
}
