//! Lockfile binding: every dependency resolution this repo drives resolves
//! against a lockfile it already has.
//!
//! `Cargo.lock` records which graph is in force, and the `provenance` module
//! in `src/main.rs` asserts that record is trustworthy — every entry from
//! crates.io, every entry checksummed. Both are statements about the *file*.
//! Neither says anything about whether a build reads it: a `cargo build`
//! without `--locked` re-resolves and rewrites the lockfile in place, so the
//! graph that was verified and the graph that compiled can differ with
//! nothing failing anywhere. This test is the missing half — the lockfile is
//! binding, not merely present — and it makes the same statement about
//! `bun.lock` via `--frozen-lockfile`.
//!
//! Scope is every file that can launch a resolution: the `Justfile`, bacon's
//! job table (behind `just watch`), the `Dockerfile`'s tool installs, and the
//! workflows. `lefthook.yml` and `docker-compose.yml` are deliberately absent
//! — they only ever call `just`, so the `Justfile` already covers them.
//!
//! This is a test rather than a `just` recipe for the reason `provenance` is:
//! it is decidable from files in the repo, so it needs no gate of its own to
//! argue with, and `just ci` runs it.

#![forbid(unsafe_code)]

use std::fs;
use std::mem;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// the rules
// ---------------------------------------------------------------------------

/// cargo sub-commands that resolve the dependency graph. Each invocation of
/// one has to name the lockfile it resolves against, or it is free to write a
/// new one.
///
/// `install` and `binstall` are in: `--locked` is what makes a tool build
/// from the lockfile its author shipped rather than from whatever the
/// registry holds the day the image is baked.
const CARGO_RESOLVING: &[&str] = &[
    "add", "bench", "binstall", "build", "check", "clean", "clippy", "deny", "doc", "install",
    "llvm-cov", "metadata", "nextest", "publish", "release", "remove", "rustc", "run", "shear",
    "test", "tree", "udeps",
];

/// cargo sub-commands that carry no `--locked`, each with the reason it is
/// already covered without one. Written down rather than skipped by pattern:
/// an exemption is a claim about a tool, and a claim wants a sentence.
const CARGO_SELF_LOCKING: &[(&str, &str)] = &[
    ("fmt", "rewrites source; it never resolves the graph"),
    ("insta", "reviews snapshots of an already-built test binary"),
    (
        "audit",
        "`Cargo.lock` is its input — it cannot audit a different one",
    ),
    ("semver-checks", "has no `--locked` in its CLI"),
    (
        "fuzz",
        "the fuzz crate is its own workspace and its `Cargo.lock` is \
         git-ignored on purpose, so `--locked` would fail on a fresh clone \
         rather than assert anything",
    ),
    ("update", "rewriting the lockfile is the whole point of it"),
];

/// JS package managers, and the sub-commands of theirs that can rewrite
/// `bun.lock`.
const JS_MANAGERS: &[&str] = &["bun", "npm", "pnpm", "yarn"];
const JS_INSTALL_SUBCOMMANDS: &[&str] = &["add", "ci", "i", "install"];

/// The flags that make a JS install refuse to rewrite its lockfile. `bun` is
/// the pinned manager here (`verify-version-pins` holds its version in three
/// places), so a call to another one fails this list on purpose.
const FROZEN_JS_FLAGS: &[&str] = &["--frozen-lockfile", "--immutable"];

// ---------------------------------------------------------------------------
// reading a command out of a config file
// ---------------------------------------------------------------------------

/// How a file spells a command that spans lines. `TomlArrays` additionally
/// joins an unclosed `[ … ]`, which is how `bacon.toml` writes a job.
#[derive(Clone, Copy)]
enum Syntax {
    Lines,
    TomlArrays,
}

/// One command: the tokens between two shell separators, and the line its
/// first token started on.
struct Command {
    line: usize,
    tokens: Vec<String>,
}

impl Command {
    fn has(&self, flag: &str) -> bool {
        self.tokens.iter().any(|token| token == flag)
    }

    /// Everything after a bare `--`, i.e. what this command hands to the tool
    /// it shells out to.
    fn passthrough(&self) -> &[String] {
        self.tokens
            .iter()
            .position(|token| token == "--")
            .map_or(&[][..], |at| &self.tokens[at..])
    }

    /// The command, short enough to read in a failure message.
    fn rendered(&self) -> String {
        let joined = self.tokens.join(" ");
        let Some((cut, _)) = joined.char_indices().nth(140) else {
            return joined;
        };
        format!("{}…", &joined[..cut])
    }
}

/// Drop a `#` comment. Only a `#` that opens a word is one, so a `#` inside
/// an argument (`grep -q "#${sha}"`) survives.
fn strip_comment(line: &str) -> &str {
    let mut after_space = true;
    for (at, ch) in line.char_indices() {
        if ch == '#' && after_space {
            return &line[..at];
        }
        after_space = ch.is_whitespace();
    }
    line
}

/// Physical lines folded into logical ones: a trailing `\` continues, and
/// under [`Syntax::TomlArrays`] so does an unclosed `[`.
fn logical_lines(text: &str, syntax: Syntax) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut start = 0;
    let mut depth = 0usize;
    for (index, raw) in text.lines().enumerate() {
        let stripped = strip_comment(raw).trim_end();
        let continued = stripped.ends_with('\\');
        let body = stripped.strip_suffix('\\').unwrap_or(stripped);
        if buf.is_empty() {
            start = index + 1;
        } else {
            buf.push(' ');
        }
        buf.push_str(body);
        if matches!(syntax, Syntax::TomlArrays) {
            depth = depth
                .saturating_add(body.matches('[').count())
                .saturating_sub(body.matches(']').count());
        }
        if continued || depth > 0 {
            continue;
        }
        if buf.trim().is_empty() {
            buf.clear();
        } else {
            out.push((start, mem::take(&mut buf)));
        }
    }
    if !buf.trim().is_empty() {
        out.push((start, buf));
    }
    out
}

/// Quoting and grouping punctuation are separators, not part of a word: it is
/// what lets one reader see `cargo` inside `bash -c '… && cargo test'`, inside
/// `out=$(cargo publish …)` and inside a TOML `["cargo", "test"]` alike.
fn tokenize(line: &str) -> Vec<String> {
    let flattened: String = line
        .chars()
        .map(|ch| {
            if matches!(
                ch,
                '"' | '\'' | '`' | '(' | ')' | '[' | ']' | '{' | '}' | ','
            ) {
                ' '
            } else {
                ch
            }
        })
        .collect();
    flattened
        .replace(';', " ; ")
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// Split a file into commands. A flag only counts for the command it sits in,
/// so `cargo build && cargo run --locked` is one bound call and one unbound.
fn commands(text: &str, syntax: Syntax) -> Vec<Command> {
    let mut out = Vec::new();
    for (line, logical) in logical_lines(text, syntax) {
        let mut tokens = Vec::new();
        for token in tokenize(&logical) {
            if matches!(token.as_str(), "&&" | "||" | ";" | "|") {
                if !tokens.is_empty() {
                    out.push(Command {
                        line,
                        tokens: mem::take(&mut tokens),
                    });
                }
            } else {
                tokens.push(token);
            }
        }
        if !tokens.is_empty() {
            out.push(Command { line, tokens });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the verdict
// ---------------------------------------------------------------------------

enum Verdict {
    /// The token was the word `cargo`, not a call: `command -v cargo`, prose,
    /// a path. Or a sub-command that resolves nothing.
    NotAnInvocation,
    Resolves,
    Unbound(String),
}

/// A sub-command reads as one only if it is a bare word — the token after
/// `cargo` in `command -v cargo > /dev/null` is `>`, and that is not a call.
fn is_subcommand_word(token: &str) -> bool {
    token.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

fn cargo_verdict(command: &Command, at: usize) -> Verdict {
    // `cargo +nightly udeps` — the toolchain prefix is not the sub-command.
    let mut next = at + 1;
    while command
        .tokens
        .get(next)
        .is_some_and(|token| token.starts_with('+'))
    {
        next += 1;
    }
    let Some(sub) = command
        .tokens
        .get(next)
        .filter(|token| is_subcommand_word(token))
    else {
        return Verdict::NotAnInvocation;
    };
    if CARGO_SELF_LOCKING.iter().any(|&(name, _)| name == sub) {
        return Verdict::NotAnInvocation;
    }
    if !CARGO_RESOLVING.contains(&sub.as_str()) {
        return Verdict::Unbound(format!(
            "`cargo {sub}` is in neither CARGO_RESOLVING nor CARGO_SELF_LOCKING. \
             Say which it is: a resolving sub-command needs `--locked`, an exempt \
             one needs the sentence saying why it is covered without it"
        ));
    }
    if command.has("--locked") {
        Verdict::Resolves
    } else {
        Verdict::Unbound(format!(
            "`cargo {sub}` without `--locked`: it re-resolves and may rewrite \
             `Cargo.lock`, so this build's graph is not the one the lockfile records"
        ))
    }
}

fn wasm_pack_verdict(command: &Command, at: usize) -> Verdict {
    if command.tokens.get(at + 1).map(String::as_str) != Some("build") {
        return Verdict::NotAnInvocation;
    }
    if command
        .passthrough()
        .iter()
        .any(|token| token == "--locked")
    {
        Verdict::Resolves
    } else {
        Verdict::Unbound(
            "`wasm-pack build` without a `-- --locked` passthrough: wasm-pack shells \
             out to `cargo build`, and only what follows `--` reaches it"
                .to_owned(),
        )
    }
}

fn js_install_verdict(command: &Command, at: usize) -> Verdict {
    let Some(sub) = command
        .tokens
        .get(at + 1)
        .filter(|token| JS_INSTALL_SUBCOMMANDS.contains(&token.as_str()))
    else {
        return Verdict::NotAnInvocation;
    };
    if FROZEN_JS_FLAGS.iter().any(|&flag| command.has(flag)) {
        Verdict::Resolves
    } else {
        Verdict::Unbound(format!(
            "`{} {sub}` without `--frozen-lockfile`: it may rewrite `bun.lock`, so \
             the tree that was tested is not the tree that was published",
            command.tokens[at]
        ))
    }
}

#[derive(Default)]
struct Scan {
    /// Resolutions found, bound or not. A parser that goes blind reports no
    /// failures, so the count is asserted too.
    resolutions: usize,
    unbound: Vec<String>,
}

fn scan(label: &str, text: &str, syntax: Syntax) -> Scan {
    let mut out = Scan::default();
    for command in commands(text, syntax) {
        for (at, token) in command.tokens.iter().enumerate() {
            let verdict = match token.as_str() {
                "cargo" => cargo_verdict(&command, at),
                "wasm-pack" => wasm_pack_verdict(&command, at),
                other if JS_MANAGERS.contains(&other) => js_install_verdict(&command, at),
                _ => Verdict::NotAnInvocation,
            };
            match verdict {
                Verdict::NotAnInvocation => {}
                Verdict::Resolves => out.resolutions += 1,
                Verdict::Unbound(why) => {
                    out.resolutions += 1;
                    out.unbound.push(format!(
                        "{label}:{}: {why}\n      {}",
                        command.line,
                        command.rendered()
                    ));
                }
            }
        }
    }
    out
}

// ---------------------------------------------------------------------------
// the repo
// ---------------------------------------------------------------------------

/// A floor under what the scan must find per file, so it cannot pass by
/// having stopped reading. A normalisation change, or a recipe rewritten into
/// a shape the tokenizer cannot see, otherwise turns this whole file green.
const RESOLUTION_FLOOR: &[(&str, usize)] = &[
    ("Justfile", 30),
    ("bacon.toml", 3),
    ("Dockerfile", 8),
    (".github/workflows/ci.yml", 1),
    (".github/workflows/docs.yml", 3),
    (".github/workflows/publish-crates.yml", 2),
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn yaml_files_in(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|ext| ext == "yml" || ext == "yaml")
        })
        .collect();
    out.sort();
    out
}

/// Every file in the repo that can launch a dependency resolution.
fn scanned_files(root: &Path) -> Vec<PathBuf> {
    let mut files = vec![
        root.join("Justfile"),
        root.join("bacon.toml"),
        root.join("Dockerfile"),
    ];
    let workflows = root.join(".github/workflows");
    assert!(workflows.is_dir(), "{} is gone", workflows.display());
    files.extend(yaml_files_in(&workflows));
    // Composite actions: one action.yml per directory, and there may be none.
    for entry in fs::read_dir(root.join(".github/actions"))
        .into_iter()
        .flatten()
        .flatten()
    {
        files.extend(yaml_files_in(&entry.path()));
    }
    files
}

fn syntax_of(path: &Path) -> Syntax {
    if path.extension().is_some_and(|ext| ext == "toml") {
        Syntax::TomlArrays
    } else {
        Syntax::Lines
    }
}

#[test]
fn every_dependency_resolution_this_repo_drives_is_bound_to_a_lockfile() {
    let root = repo_root();
    let mut unbound = Vec::new();
    let mut found: Vec<(String, usize)> = Vec::new();

    for path in scanned_files(&root) {
        let label = path
            .strip_prefix(&root)
            .unwrap_or(path.as_path())
            .display()
            .to_string();
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        let scanned = scan(&label, &text, syntax_of(&path));
        found.push((label, scanned.resolutions));
        unbound.extend(scanned.unbound);
    }

    for &(file, floor) in RESOLUTION_FLOOR {
        let seen = found
            .iter()
            .find(|(label, _)| label == file)
            .map_or(0, |&(_, count)| count);
        assert!(
            seen >= floor,
            "{file}: {seen} dependency resolutions found, expected at least {floor}. \
             The scan is reading less of the file than it used to, so its silence \
             means nothing."
        );
    }

    assert!(
        unbound.is_empty(),
        "dependency resolutions not bound to a lockfile:\n{}",
        unbound.join("\n")
    );
}

// ---------------------------------------------------------------------------
// what the scan claims, pinned both ways
// ---------------------------------------------------------------------------

#[test]
fn a_resolving_call_without_the_flag_is_reported_and_with_it_is_not() {
    let bare = scan(
        "Justfile",
        "    {{_dev}} cargo build --workspace\n",
        Syntax::Lines,
    );
    assert_eq!(bare.resolutions, 1);
    assert_eq!(bare.unbound.len(), 1, "{:?}", bare.unbound);
    assert!(
        bare.unbound[0].contains("cargo build"),
        "{:?}",
        bare.unbound
    );

    let bound = scan(
        "Justfile",
        "    {{_dev}} cargo build --locked --workspace\n",
        Syntax::Lines,
    );
    assert_eq!(bound.resolutions, 1);
    assert!(bound.unbound.is_empty(), "{:?}", bound.unbound);
}

#[test]
fn a_flag_on_the_neighbouring_command_does_not_cover_this_one() {
    // The reason a command, not a line, is the unit: one `--locked` anywhere
    // on the line would otherwise absolve everything else on it.
    let scanned = scan(
        "Justfile",
        "    {{_dev}} bash -c 'cargo build && cargo run --locked -- x'\n",
        Syntax::Lines,
    );
    assert_eq!(scanned.resolutions, 2);
    assert_eq!(scanned.unbound.len(), 1, "{:?}", scanned.unbound);
    assert!(
        scanned.unbound[0].contains("cargo build"),
        "{:?}",
        scanned.unbound
    );
}

#[test]
fn a_toolchain_prefix_does_not_hide_the_subcommand() {
    let bare = scan(
        "Justfile",
        "    {{_fuzz}} cargo +nightly udeps --workspace --all-targets\n",
        Syntax::Lines,
    );
    assert_eq!(bare.unbound.len(), 1, "{:?}", bare.unbound);

    let exempt = scan(
        "Justfile",
        "    {{_fuzz}} bash -c 'cd crates/x && cargo +nightly fuzz run roundtrip'\n",
        Syntax::Lines,
    );
    assert_eq!(exempt.resolutions, 0);
    assert!(exempt.unbound.is_empty(), "{:?}", exempt.unbound);
}

#[test]
fn a_flag_on_a_continuation_line_is_still_this_commands_flag() {
    // `just coverage` spells it this way; a line-at-a-time reader would call
    // this unbound and be wrong, which is the failure that gets a gate turned off.
    let scanned = scan(
        "Justfile",
        "    {{_dev}} cargo llvm-cov nextest \\\n        --locked \\\n        --workspace\n",
        Syntax::Lines,
    );
    assert_eq!(scanned.resolutions, 1);
    assert!(scanned.unbound.is_empty(), "{:?}", scanned.unbound);
}

#[test]
fn a_bacon_job_is_read_as_one_command_across_its_toml_array() {
    let job = "[jobs.test]\ncommand = [\n    \"cargo\", \"nextest\", \"run\",\n    \"--locked\", \"--workspace\",\n]\n";
    let joined = scan("bacon.toml", job, Syntax::TomlArrays);
    assert_eq!(joined.resolutions, 1);
    assert!(joined.unbound.is_empty(), "{:?}", joined.unbound);

    // Same text read line-at-a-time: the flag is on another line, so the call
    // reads as unbound. That gap is why the syntax is per-file, not global.
    let split = scan("bacon.toml", job, Syntax::Lines);
    assert_eq!(split.unbound.len(), 1, "{:?}", split.unbound);
}

#[test]
fn wasm_pack_needs_the_flag_where_the_cargo_it_shells_out_to_can_see_it() {
    let kept = scan(
        "Justfile",
        "wasm-pack build crates/x --locked --target bundler\n",
        Syntax::Lines,
    );
    assert_eq!(kept.unbound.len(), 1, "{:?}", kept.unbound);

    let passed = scan(
        "Justfile",
        "wasm-pack build crates/x --target bundler -- --locked\n",
        Syntax::Lines,
    );
    assert!(passed.unbound.is_empty(), "{:?}", passed.unbound);
}

#[test]
fn a_js_install_must_be_frozen_and_a_script_run_is_not_an_install() {
    let bare = scan(
        "docs.yml",
        "        run: cd playground && bun install\n",
        Syntax::Lines,
    );
    assert_eq!(bare.unbound.len(), 1, "{:?}", bare.unbound);
    assert!(bare.unbound[0].contains("bun.lock"), "{:?}", bare.unbound);

    let frozen = scan(
        "docs.yml",
        "        run: cd playground && bun install --frozen-lockfile\n",
        Syntax::Lines,
    );
    assert!(frozen.unbound.is_empty(), "{:?}", frozen.unbound);

    // `bun run build` resolves nothing — it runs a script that is already installed.
    let script = scan(
        "docs.yml",
        "        run: cd playground && bun run build\n",
        Syntax::Lines,
    );
    assert_eq!(script.resolutions, 0);
    assert!(script.unbound.is_empty(), "{:?}", script.unbound);
}

#[test]
fn a_subcommand_that_takes_no_locked_flag_is_exempt() {
    for &(sub, why) in CARGO_SELF_LOCKING {
        let scanned = scan(
            "Justfile",
            &format!("    {{{{_dev}}}} cargo {sub} --all\n"),
            Syntax::Lines,
        );
        assert_eq!(scanned.resolutions, 0, "cargo {sub} ({why})");
        assert!(
            scanned.unbound.is_empty(),
            "cargo {sub}: {:?}",
            scanned.unbound
        );
    }
}

#[test]
fn an_unclassified_subcommand_fails_rather_than_passing_silently() {
    // Even carrying the flag: an unknown sub-command is an unanswered question
    // about whether the flag is the right answer for it, and the answer belongs
    // in one of the two tables where the next reader will find it.
    let scanned = scan(
        "Justfile",
        "    {{_dev}} cargo vendor --locked\n",
        Syntax::Lines,
    );
    assert_eq!(scanned.unbound.len(), 1, "{:?}", scanned.unbound);
    assert!(
        scanned.unbound[0].contains("cargo vendor"),
        "{:?}",
        scanned.unbound
    );
}

#[test]
fn a_mention_of_cargo_that_is_not_an_invocation_is_ignored() {
    // Every shape that appears in the scanned files and must stay silent:
    // release.yml's PATH probe, a cargo home path, and a comment.
    let text = "          if ! command -v cargo > /dev/null 2>&1; then\n\
                \x20           echo \"$HOME/.cargo/bin\" >> $GITHUB_PATH\n\
                # rerun `cargo build` by hand after this\n";
    let scanned = scan("release.yml", text, Syntax::Lines);
    assert_eq!(scanned.resolutions, 0, "{:?}", scanned.unbound);
    assert!(scanned.unbound.is_empty(), "{:?}", scanned.unbound);
}

#[test]
fn a_hash_inside_an_argument_does_not_truncate_the_command() {
    // Comment stripping is how prose stays out of the scan; if it were greedy
    // it would also swallow the flag off the end of a real command.
    let scanned = scan(
        "Justfile",
        "    {{_dev}} cargo run --package xtask -- --sentinel=#42 --locked\n",
        Syntax::Lines,
    );
    assert_eq!(scanned.resolutions, 1);
    assert!(scanned.unbound.is_empty(), "{:?}", scanned.unbound);
}
