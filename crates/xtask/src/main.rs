//! Workspace automation: every task the `Justfile` or CI invokes that is not
//! a direct cargo call. `--help` lists the sub-commands.
//!
//! Aozora parser / corpus concerns live in the sibling `P4suta/aozora` repo
//! (ADR-0010), along with their refresh sub-commands.

#![forbid(unsafe_code)]
#![allow(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "xtask is a command-line program whose generated output and status belong on the console"
)]

use std::env;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::Command as ProcessCommand;

use anyhow::{Context, Result, bail};
use clap::{Parser, Subcommand};
use encoding_rs::SHIFT_JIS;

mod spec_refresh;

#[derive(Parser, Debug)]
#[command(version, about = "aozora-flavored-markdown workspace automation", long_about = None)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
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
    /// Project every UTF-8 file in a directory to the exact CP932 byte
    /// sequence used by the fuzz corpus, dropping only unmappable scalars.
    Cp932Project {
        /// Directory containing UTF-8 source documents.
        #[arg(long)]
        input_dir: PathBuf,
        /// Directory that receives representable CP932 documents.
        #[arg(long)]
        output_dir: PathBuf,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command {
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
        Command::Cp932Project {
            input_dir,
            output_dir,
        } => cp932_project(&input_dir, &output_dir),
    }
}

/// Encode a seed directory through `encoding_rs`, the same WHATWG Shift_JIS
/// implementation used by the `aozora` decoder. Every source document is
/// emitted. A scalar CP932 cannot represent is dropped rather than replacing
/// or omitting the whole document, so the corpus remains the complete
/// character-wise projection of the UTF-8 source set.
fn cp932_project(input_dir: &Path, output_dir: &Path) -> Result<()> {
    fs::create_dir_all(output_dir).with_context(|| format!("mkdir {}", output_dir.display()))?;
    let mut entries = fs::read_dir(input_dir)
        .with_context(|| format!("reading {}", input_dir.display()))?
        .collect::<io::Result<Vec<_>>>()?;
    entries.sort_by_key(fs::DirEntry::file_name);

    let mut written = 0usize;
    let mut dropped_scalars = 0usize;
    let mut affected_documents = 0usize;
    for entry in entries {
        let path = entry.path();
        if path.is_file() {
            let source =
                fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
            let mut encoded = Vec::with_capacity(source.len());
            let mut dropped_here = 0usize;
            for ch in source.chars() {
                let mut utf8 = [0; 4];
                let scalar = ch.encode_utf8(&mut utf8);
                let (bytes, _, had_errors) = SHIFT_JIS.encode(scalar);
                if had_errors {
                    dropped_here += 1;
                } else {
                    encoded.extend_from_slice(bytes.as_ref());
                }
            }
            if dropped_here > 0 {
                affected_documents += 1;
                dropped_scalars += dropped_here;
            }
            let output = output_dir.join(entry.file_name());
            fs::write(&output, &encoded)
                .with_context(|| format!("writing {}", output.display()))?;
            written += 1;
        }
    }
    println!(
        "cp932-project: wrote {written} document(s); dropped {dropped_scalars} unrepresentable \
         scalar(s) from {affected_documents} document(s)"
    );
    Ok(())
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
#[allow(
    clippy::expect_used,
    reason = "the version-pin regular expression is a compile-time constant covered by this command's tests"
)]
fn aozora_pin_pattern() -> regex::Regex {
    regex::Regex::new(r#"(?m)^(aozora\s*=\s*\{\s*version\s*=\s*"=?)(\d+\.\d+\.\d+)(")"#)
        .expect("aozora version pattern compiles")
}

fn aozora_bump(new_version: &str) -> Result<()> {
    // Accept only a fully-spelled `major.minor.patch` so both manifests keep
    // an unambiguous, reviewable version requirement.
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

    // Refresh both committed lockfiles. The fuzz crate is a separate cargo
    // workspace, so it needs its own explicit update.
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
    let fuzz_status = ProcessCommand::new("cargo")
        .args([
            "update",
            "--manifest-path",
            "crates/aozora-flavored-markdown/fuzz/Cargo.toml",
            "-p",
            "aozora",
        ])
        .status()
        .context("invoking cargo update for the fuzz workspace")?;
    if !fuzz_status.success() {
        bail!(
            "fuzz cargo update exited with {fuzz_status:?}. The manifests and root lockfile \
             were rewritten; re-run the fuzz-workspace update after fixing the fetch or \
             network issue."
        );
    }
    println!("aozora-bump: both Cargo.lock files refreshed against {new_version}");
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
