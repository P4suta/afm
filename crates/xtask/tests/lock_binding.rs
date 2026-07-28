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

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
// Aliased: `Command` in this file is the shell command a config line spells,
// which is the other half of the same word.
use std::process::Command as Spawn;

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
    "llvm-cov", "metadata", "nextest", "pkgid", "publish", "remove", "rustc", "run", "shear",
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
        "cargo-fuzz 0.13.2 has no `--locked` and no way to pass one to the \
         `cargo build` it shells out to. The fuzz crate's `Cargo.lock` is \
         committed instead (DEV-293), `just fuzz-build` fails when a build \
         rewrote it — the same refusal, one step later — and \
         `just verify-version-pins` compares the versions the two lockfiles \
         resolve to",
    ),
    ("update", "rewriting the lockfile is the whole point of it"),
    (
        "release",
        "the same answer as `update`, one level up: `just release` bumps every \
         member's version, and `Cargo.lock` records those versions, so rewriting \
         it is what the step is for. cargo-release 1.1.3 has no `--locked` to \
         pass anyway",
    ),
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

    /// Everything before a bare `--`, i.e. what this command keeps for itself.
    fn own_args(&self) -> &[String] {
        self.tokens
            .iter()
            .position(|token| token == "--")
            .map_or(&self.tokens[..], |at| &self.tokens[..at])
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

/// Where a wasm-pack sub-command's cargo flags have to sit.
#[derive(Clone, Copy)]
enum CargoFlagsGo {
    /// `wasm-pack build` forwards what follows a bare `--` to `cargo build`.
    AfterDoubleDash,
    /// `wasm-pack test` takes cargo's flags as trailing positionals
    /// (`PATH_AND_EXTRA_OPTIONS`) and hands a `--` on to the *test binary*,
    /// where `--locked` is not an argument at all. Writing the build form here
    /// fails with "unexpected argument" from wasm-bindgen-test-runner, which is
    /// loud — but the reader has to know the difference or it would bless the
    /// unbound shape `wasm-pack test … --node` on the way past.
    AsPositionals,
}

/// The two wasm-pack sub-commands that shell out to cargo. `test` is here for
/// the same reason `build` is, and was missing for as long as no recipe spelled
/// it: an unlisted sub-command reads as "not an invocation", so `just test-wasm`
/// would have been exempt by silence rather than by argument.
const WASM_PACK_RESOLVING: &[(&str, CargoFlagsGo)] = &[
    ("build", CargoFlagsGo::AfterDoubleDash),
    ("test", CargoFlagsGo::AsPositionals),
];

fn wasm_pack_verdict(command: &Command, at: usize) -> Verdict {
    let Some(&(sub, where_flags_go)) = command
        .tokens
        .get(at + 1)
        .and_then(|token| WASM_PACK_RESOLVING.iter().find(|&&(name, _)| name == token))
    else {
        return Verdict::NotAnInvocation;
    };
    let seen = match where_flags_go {
        CargoFlagsGo::AfterDoubleDash => command.passthrough(),
        CargoFlagsGo::AsPositionals => command.own_args(),
    };
    if seen.iter().any(|token| token == "--locked") {
        Verdict::Resolves
    } else {
        Verdict::Unbound(format!(
            "`wasm-pack {sub}` without a `--locked` the cargo it shells out to can see: \
             `build` forwards what follows a `--`, `test` takes cargo's flags as trailing \
             positionals and gives the `--` to the test binary instead"
        ))
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
    // ci.yml is absent on purpose: it no longer spells a cargo call at all.
    // Its one bare `cargo check` was the msrv job's, and that job now runs
    // `just msrv` — the same recipe `just ci` runs — so the flag it needs is
    // the Justfile's, where the floor above already covers it.
    //
    // docs.yml is absent for the same reason: it fell from 3 to 2 when its
    // `cargo doc` became `just doc`, and to none when the wasm-pack build and
    // the bun install became `just playground-build` (DEV-310). Every flag it
    // needs is in the Justfile, where the floor above covers it.
    //
    // publish-crates.yml went the same way, 2 to 1: its preflight dry run is
    // `just package` now (DEV-224), so the flag on that resolution is the
    // Justfile's. What is left here is the upload, and that one stays — no
    // recipe may push to crates.io, so it is spelled where it runs and its
    // `--locked` has to be read here.
    (".github/workflows/publish-crates.yml", 1),
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

    // `test` reaches cargo too, and reading only `build` is how a `wasm-pack
    // test` recipe would have been exempt without anyone deciding it should be.
    // Its flags are trailing positionals, so the two sub-commands want the flag
    // in DIFFERENT places and each rejects the other's spelling.
    let tested = scan(
        "Justfile",
        "wasm-pack test --node crates/x --locked\n",
        Syntax::Lines,
    );
    assert_eq!(tested.resolutions, 1);
    assert!(tested.unbound.is_empty(), "{:?}", tested.unbound);

    let after_dashes = scan(
        "Justfile",
        "wasm-pack test --node crates/x -- --locked\n",
        Syntax::Lines,
    );
    assert_eq!(after_dashes.unbound.len(), 1, "{:?}", after_dashes.unbound);
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

// ---------------------------------------------------------------------------
// the one exemption that promises machinery instead of describing a CLI
// ---------------------------------------------------------------------------

/// The lockfile the fuzz workspace resolves against.
const FUZZ_LOCK: &str = "crates/aozora-flavored-markdown/fuzz/Cargo.lock";

/// The recipe that re-resolves [`FUZZ_LOCK`] onto the graph the workspace
/// ships — the one command that turns this drift green.
///
/// Named once and interpolated into the failure below, so the instruction a
/// person is given and the name this file checks against the Justfile cannot
/// become two different words.
const FUZZ_LOCK_REPAIR: &str = "fuzz-lock";

/// The one dependency the fuzz workspace has and this workspace does not.
/// libFuzzer's bindings are nightly-only, which is the entire reason the fuzz
/// crate declares a `[workspace]` of its own — so everything it drags in is
/// legitimately absent from `Cargo.lock`, and nothing else is.
const LIBFUZZER: &str = "libfuzzer-sys";

/// The versions a lockfile resolves each package to. A name can appear more
/// than once — cargo keeps two incompatible majors side by side — so the
/// answer is a set, and a subset relation rather than equality is what
/// "resolves the same graph" means between two workspaces of different size.
fn locked_versions(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut name: Option<String> = None;
    for line in text.lines() {
        let body = line.trim();
        if body == "[[package]]" {
            name = None;
        } else if let Some(value) = body.strip_prefix("name = ") {
            name = Some(value.trim_matches('"').to_owned());
        } else if let Some(value) = body.strip_prefix("version = ") {
            // The `version = 4` at the top of the file is the lockfile format,
            // and it is the one `version` with no `name` above it.
            if let Some(name) = name.take() {
                out.entry(name)
                    .or_default()
                    .insert(value.trim_matches('"').to_owned());
            }
        }
    }
    out
}

/// What each package in a lockfile depends on, by name. cargo spells an entry
/// as `"name"` or `"name version"`, and only the name is the link.
fn locked_dependencies(text: &str) -> BTreeMap<String, BTreeSet<String>> {
    let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut name = String::new();
    let mut inside = false;
    for line in text.lines() {
        let body = line.trim();
        if body == "[[package]]" {
            name.clear();
            inside = false;
        } else if let Some(value) = body.strip_prefix("name = ") {
            value.trim_matches('"').clone_into(&mut name);
        } else if let Some(rest) = body.strip_prefix("dependencies = [") {
            inside = !rest.trim_end().ends_with(']');
        } else if inside {
            if body == "]" {
                inside = false;
            } else if let Some(dep) = body
                .trim_matches(|ch| ch == '"' || ch == ',')
                .split_whitespace()
                .next()
            {
                out.entry(name.clone()).or_default().insert(dep.to_owned());
            }
        }
    }
    out
}

/// Everything reachable from `root` in a lockfile's graph, `root` included.
fn reachable(graph: &BTreeMap<String, BTreeSet<String>>, root: &str) -> BTreeSet<String> {
    let mut seen = BTreeSet::new();
    let mut stack = vec![root.to_owned()];
    while let Some(name) = stack.pop() {
        if !seen.insert(name.clone()) {
            continue;
        }
        if let Some(deps) = graph.get(&name) {
            stack.extend(deps.iter().cloned());
        }
    }
    seen
}

/// The packages nothing in the lockfile depends on: the crate it was resolved
/// for. Derived rather than named, so it stays right when the crate is
/// renamed.
fn lockfile_roots(
    graph: &BTreeMap<String, BTreeSet<String>>,
    packages: &BTreeSet<&str>,
) -> Vec<String> {
    let depended: BTreeSet<&str> = graph.values().flatten().map(String::as_str).collect();
    packages
        .iter()
        .filter(|name| !depended.contains(*name))
        .map(|name| (*name).to_owned())
        .collect()
}

/// Every way the fuzz lockfile can disagree with the workspace's about the
/// graph the fuzz targets link.
///
/// Two shapes, and the second is the one a reader that compared only the names
/// both files hold could not see. A package the workspace no longer resolves
/// AT ALL — comrak dropping a dependency, a crate renamed upstream — leaves the
/// stale fuzz lockfile as the only file in the repo still holding it, and a
/// shared-names-only comparison drops it on the way in: the targets go on
/// fuzzing code this repo does not ship, which is the sentence the assertion
/// below has always failed with. The legitimate absentees are derivable rather
/// than listable — everything under [`LIBFUZZER`], plus the fuzz crate itself.
fn fuzz_lock_drift(fuzz_text: &str, workspace_text: &str) -> Vec<String> {
    let fuzz = locked_versions(fuzz_text);
    let workspace = locked_versions(workspace_text);
    let graph = locked_dependencies(fuzz_text);
    let names: BTreeSet<&str> = fuzz.keys().map(String::as_str).collect();
    let tail = reachable(&graph, LIBFUZZER);
    let roots = lockfile_roots(&graph, &names);

    let mut out = Vec::new();
    for (name, theirs) in &fuzz {
        match workspace.get(name) {
            Some(ours) if theirs.is_subset(ours) => {}
            Some(ours) => out.push(format!(
                "  {name}: fuzz {theirs:?} vs workspace {ours:?} — a pin left behind"
            )),
            None if tail.contains(name) || roots.contains(name) => {}
            None => out.push(format!(
                "  {name} {theirs:?}: in the fuzz graph, in no workspace one, and not part of \
                 `{LIBFUZZER}`'s tail — the workspace stopped resolving it and this file did not"
            )),
        }
    }
    out
}

#[test]
fn the_lockfile_that_stands_in_for_the_fuzz_exemption_resolves_the_graph_the_workspace_ships() {
    // Every other entry in CARGO_SELF_LOCKING states a fact about a tool: this
    // sub-command rewrites source, that one takes the lockfile as its input.
    // `fuzz` is the only one whose reason promises machinery — a committed
    // lockfile, a `just fuzz-build` that fails when a build rewrote it, a
    // `just verify-version-pins` that compares what the two lockfiles resolve
    // — and machinery promised in a comment is what the whole file exists to
    // reject. Nothing checked it. The exemption was even accurate before this
    // PR under an opposite arrangement (the lockfile was git-ignored on
    // purpose), so the sentence changing is not something a reader would catch
    // either.
    //
    // Measured here rather than left to the recipe, and measured wider than the
    // recipe measures: `verify-version-pins` compares two names, `aozora` and
    // `comrak`. The fuzz targets link the whole graph. `comrak` pinned to the
    // same version while `comrak`'s own parser dependency resolved differently
    // is a target fuzzing a parser this workspace does not ship, with both
    // manifests and both named pins in perfect agreement.
    let root = repo_root();
    let fuzz_lock = root.join(FUZZ_LOCK);
    let fuzz = fs::read_to_string(&fuzz_lock).unwrap_or_else(|e| {
        panic!(
            "reading {FUZZ_LOCK}: {e}\n\
             The fuzz crate is its own workspace, so it resolves its own graph, and cargo-fuzz \
             has no `--locked` to bind that resolution with. Without this file the targets \
             re-resolve on every build and nothing in the repo can say what they were built \
             against (DEV-293)."
        )
    });
    let workspace = fs::read_to_string(root.join("Cargo.lock"))
        .unwrap_or_else(|e| panic!("reading Cargo.lock: {e}"));

    let versions = locked_versions(&fuzz);
    let workspace_versions = locked_versions(&workspace);
    let shared = versions
        .keys()
        .filter(|name| workspace_versions.contains_key(*name))
        .count();
    assert!(
        shared >= 40 && workspace_versions.len() >= 100,
        "{} packages in the fuzz lockfile, {} in the workspace's, {shared} shared; the reader is \
         not finding the `[[package]]` blocks",
        versions.len(),
        workspace_versions.len(),
    );

    // The other half of the same reader, and the one the absentee rule rests
    // on: a graph nobody can walk makes every fuzz-only package look like the
    // libFuzzer tail, i.e. makes that rule pass on everything.
    let graph = locked_dependencies(&fuzz);
    let names: BTreeSet<&str> = versions.keys().map(String::as_str).collect();
    let tail = reachable(&graph, LIBFUZZER);
    let roots = lockfile_roots(&graph, &names);
    assert!(
        tail.len() >= 3 && tail.contains(LIBFUZZER),
        "`{LIBFUZZER}` reaches {tail:?} in {FUZZ_LOCK}; the reader is not finding the \
         `dependencies` lists, so \"absent from the workspace because libFuzzer brought it\" is \
         no longer something this file can tell from \"absent because the workspace dropped it\""
    );
    assert_eq!(
        roots.len(),
        1,
        "{FUZZ_LOCK} has {roots:?} that nothing depends on. Exactly one package is the crate the \
         lockfile was resolved for; more than one means a package sits in this graph with no \
         path to a fuzz target, and none means the reader is not reading."
    );

    let drifted = fuzz_lock_drift(&fuzz, &workspace);
    assert!(
        drifted.is_empty(),
        "the fuzz targets and the library do not resolve the same graph:\n{}\n\
         A subset is fine — the fuzz graph is the smaller one, and the workspace keeps two majors \
         of a few crates for dependencies the harnesses never reach. A version the workspace does \
         not have is not, and neither is a package it no longer has at all: the targets are then \
         fuzzing code this repo does not ship. Run `just {FUZZ_LOCK_REPAIR}` and commit \
         {FUZZ_LOCK}.",
        drifted.join("\n")
    );

    // And that the two names the recipe does compare are among them, so its
    // narrower check is narrow rather than vacuous.
    for name in ["aozora", "comrak"] {
        assert!(
            versions.contains_key(name) && workspace_versions.contains_key(name),
            "`{name}` is not in both lockfiles, and `just verify-version-pins` compares exactly \
             it and one other by name — over a package one side does not have, that comparison \
             passes on two empty strings"
        );
    }
}

/// A fuzz lockfile of the shape this repo's has: the crate, libFuzzer's tail,
/// and the library's own graph.
const FUZZ_LOCK_SAMPLE: &str = concat!(
    "version = 4\n\n",
    "[[package]]\nname = \"aozora-md-fuzz\"\nversion = \"0.0.0\"\n",
    "dependencies = [\n \"aozora-flavored-markdown\",\n \"libfuzzer-sys\",\n]\n\n",
    "[[package]]\nname = \"libfuzzer-sys\"\nversion = \"0.4.13\"\n",
    "dependencies = [\n \"arbitrary\",\n \"cc\",\n]\n\n",
    "[[package]]\nname = \"arbitrary\"\nversion = \"1.4.2\"\n\n",
    "[[package]]\nname = \"cc\"\nversion = \"1.4.0\"\ndependencies = [\n \"jobserver\",\n]\n\n",
    "[[package]]\nname = \"jobserver\"\nversion = \"0.1.35\"\n\n",
    "[[package]]\nname = \"aozora-flavored-markdown\"\nversion = \"0.5.0\"\n",
    "dependencies = [\n \"comrak\",\n \"rustc-hash\",\n]\n\n",
    "[[package]]\nname = \"comrak\"\nversion = \"0.52.0\"\ndependencies = [\n \"entities\",\n]\n\n",
    "[[package]]\nname = \"entities\"\nversion = \"1.0.2\"\n\n",
    "[[package]]\nname = \"rustc-hash\"\nversion = \"2.1.3\"\n",
);

/// The workspace lockfile it agrees with: the same library graph, none of
/// libFuzzer's, and one crate resolved twice the way this workspace really
/// resolves `rustc-hash` and `unicode-width`.
const WORKSPACE_LOCK_SAMPLE: &str = concat!(
    "version = 4\n\n",
    "[[package]]\nname = \"aozora-flavored-markdown\"\nversion = \"0.5.0\"\n",
    "dependencies = [\n \"comrak\",\n \"rustc-hash\",\n]\n\n",
    "[[package]]\nname = \"comrak\"\nversion = \"0.52.0\"\ndependencies = [\n \"entities\",\n]\n\n",
    "[[package]]\nname = \"entities\"\nversion = \"1.0.2\"\n\n",
    "[[package]]\nname = \"cc\"\nversion = \"1.4.0\"\n\n",
    "[[package]]\nname = \"rustc-hash\"\nversion = \"1.1.0\"\n\n",
    "[[package]]\nname = \"rustc-hash\"\nversion = \"2.1.3\"\n",
);

#[test]
fn the_drift_reader_reports_a_bump_left_behind_and_stays_silent_on_the_shape_the_two_graphs_have() {
    // The assertion above says the two lockfiles agree today. What it cannot
    // say is that it would notice if they did not — and "the gate fails when
    // the thing it gates happens" is exactly the half that gets demonstrated
    // by hand once, in the pull request that adds the gate, and never again.
    // Both verdicts are pinned here instead.
    let agreeing = fuzz_lock_drift(FUZZ_LOCK_SAMPLE, WORKSPACE_LOCK_SAMPLE);
    assert!(
        agreeing.is_empty(),
        "the shape the two lockfiles legitimately have reads as drift: {agreeing:?}. The fuzz \
         graph is the smaller one, libFuzzer's tail is absent from the workspace's on purpose, \
         and the workspace holds two majors of a crate the harnesses reach once."
    );

    // Dependabot's #177 exactly: the workspace pin moved, the second lockfile
    // did not.
    let bumped = WORKSPACE_LOCK_SAMPLE.replace("0.52.0", "0.54.0");
    let stale = fuzz_lock_drift(FUZZ_LOCK_SAMPLE, &bumped);
    assert_eq!(stale.len(), 1, "{stale:?}");
    assert!(stale[0].contains("comrak"), "{stale:?}");

    // And the shape a comparison over shared names only could not see: comrak
    // drops a dependency, the workspace lockfile loses it, and the stale fuzz
    // lockfile is left as the only file in the repo that still resolves it.
    // The package is in neither `shared` nor libFuzzer's tail, so the reader
    // that filtered on `workspace.contains_key` dropped it before comparing
    // anything — and a fuzz target went on linking a crate this repo had
    // stopped shipping.
    let dropped = WORKSPACE_LOCK_SAMPLE
        .replace("dependencies = [\n \"entities\",\n]\n", "")
        .replace(
            "[[package]]\nname = \"entities\"\nversion = \"1.0.2\"\n\n",
            "",
        );
    assert!(
        !locked_versions(&dropped).contains_key("entities"),
        "the sample edit did not remove the package it is about"
    );
    let orphaned = fuzz_lock_drift(FUZZ_LOCK_SAMPLE, &dropped);
    assert_eq!(orphaned.len(), 1, "{orphaned:?}");
    assert!(orphaned[0].contains("entities"), "{orphaned:?}");

    // The absentee rule stays a rule and not a blanket: nothing under
    // libFuzzer is ever reported, however far down it sits. `jobserver` is
    // three edges away, through a `cc` the workspace does hold — at the same
    // version, with a shorter `dependencies` list, because only the fuzz graph
    // turns on the feature that pulls the scheduler in. The silence in the
    // agreeing case is the other half: the fuzz graph holds one `rustc-hash`
    // where the workspace holds two, and a reader comparing for equality
    // rather than for a subset would have called that drift on every run.
    for tail in ["libfuzzer-sys", "arbitrary", "jobserver", "aozora-md-fuzz"] {
        assert!(
            !stale.iter().any(|line| line.contains(tail)),
            "`{tail}` is libFuzzer's or the fuzz crate's own and was reported: {stale:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// the repair that machinery leaves a person
// ---------------------------------------------------------------------------

/// One `just` recipe: the `[attribute]` lines directly above its header, and
/// the indented lines under it.
struct Recipe<'a> {
    attributes: Vec<&'a str>,
    body: Vec<&'a str>,
}

/// Whether `line` opens the recipe called `name`. A header sits at column 0
/// and spells any parameters before its `:`; `_fuzz := …` is an assignment and
/// not one.
fn opens_recipe(line: &str, name: &str) -> bool {
    let Some(rest) = line.strip_prefix(name) else {
        return false;
    };
    match rest.chars().next() {
        Some(':') => true,
        Some(' ') => rest.contains(':') && !rest.trim_start().starts_with(":="),
        _ => false,
    }
}

fn recipe<'a>(justfile: &'a str, name: &str) -> Option<Recipe<'a>> {
    let lines: Vec<&str> = justfile.lines().collect();
    let at = lines.iter().position(|line| opens_recipe(line, name))?;
    let mut first = at;
    while first > 0 && lines[first - 1].starts_with('[') {
        first -= 1;
    }
    let body = lines[at + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || line.starts_with([' ', '\t']))
        .copied()
        .collect();
    Some(Recipe {
        attributes: lines[first..at].to_vec(),
        body,
    })
}

/// Every `just <recipe>` a sentence names. Backquoted only: "just" is an
/// English adverb, and a recipe calling another one writes no quotes.
fn recipes_named_in(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = text;
    while let Some((_, after)) = rest.split_once("`just ") {
        rest = after;
        let name: String = after
            .chars()
            .take_while(|&ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
            .collect();
        if !name.is_empty() {
            out.insert(name);
        }
    }
    out
}

/// The cargo sub-commands a script spells, toolchain prefixes skipped.
fn cargo_subcommands(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for command in commands(text, Syntax::Lines) {
        for (at, token) in command.tokens.iter().enumerate() {
            if token != "cargo" {
                continue;
            }
            let mut next = at + 1;
            while command
                .tokens
                .get(next)
                .is_some_and(|token| token.starts_with('+'))
            {
                next += 1;
            }
            if let Some(sub) = command
                .tokens
                .get(next)
                .filter(|token| is_subcommand_word(token))
            {
                out.push(sub.clone());
            }
        }
    }
    out
}

/// The entry on [`CARGO_SELF_LOCKING`] whose reason is a promise rather than a
/// description.
const MACHINERY_EXEMPTION: &str = "fuzz";

/// The gate that stands in for the flag cargo-fuzz does not have: it compiles
/// the targets and then asks whether doing so rewrote the lockfile.
const REWRITE_GATE: &str = "fuzz-build";

/// The gates that fail on this drift, each with what it compares. Their
/// failure output is where a person meets the second lockfile — usually for
/// the first time — so it is where the repair has to be spelled.
const GATES_THAT_REPORT_THIS_DRIFT: &[(&str, &str)] = &[
    (
        "verify-version-pins",
        "compares what the two lockfiles resolve `aozora` and `comrak` to",
    ),
    (REWRITE_GATE, "fails when the build it just ran rewrote it"),
];

#[test]
fn the_gate_standing_in_for_the_flag_asks_whether_the_build_rewrote_the_file() {
    // `cargo fuzz` is exempt from `--locked` because this recipe catches the
    // re-resolution a moment after the fact instead of refusing it a moment
    // before. What it caught was `git diff --quiet -- $lock`, and that is a
    // different question: "does this file differ from the last commit". The
    // two answers coincide on a clean CI checkout and nowhere else.
    //
    // Where they part is the workflow this PR institutes. `just fuzz-lock`
    // re-resolves the file and prints the diff to review; the lockfile is then
    // correct, uncommitted, and every `just ci` until the commit fails here —
    // with the file's checksum unchanged across the build, under a message
    // saying the build rewrote it. Measured on the bump this test arrived
    // with: sha256 identical before and after, gate red. A gate that is red on
    // the correct state is one people learn to scroll past, and this one is
    // the only thing standing where a flag would be.
    let justfile = fs::read_to_string(repo_root().join("Justfile"))
        .unwrap_or_else(|e| panic!("reading Justfile: {e}"));
    let gate = recipe(&justfile, REWRITE_GATE)
        .unwrap_or_else(|| panic!("`just {REWRITE_GATE}` is not a recipe"));
    let body = gate.body.join("\n");

    assert!(
        !body.contains("git diff --quiet"),
        "`just {REWRITE_GATE}` decides on `git diff --quiet`, i.e. on whether {FUZZ_LOCK} matches \
         HEAD. A build that rewrote nothing fails that on any branch carrying a re-resolution it \
         has not committed yet — and whether the file is committed is somebody else's question: \
         a clone getting it is `gate_wiring.rs`'s, the two lockfiles agreeing is \
         `verify-version-pins`'s."
    );

    let snapshot = gate
        .body
        .iter()
        .position(|line| line.contains("cp \"$lock\""));
    let compiles = gate
        .body
        .iter()
        .position(|line| line.contains("fuzz build"));
    let (snapshot, compiles) = (
        snapshot.unwrap_or_else(|| {
            panic!(
                "`just {REWRITE_GATE}` takes no copy of {FUZZ_LOCK}. \"Did this build rewrite the \
                 file\" is answerable only against what the file held before it ran."
            )
        }),
        compiles.unwrap_or_else(|| panic!("`just {REWRITE_GATE}` no longer builds the targets")),
    );
    assert!(
        snapshot < compiles,
        "`just {REWRITE_GATE}` copies {FUZZ_LOCK} at line {snapshot} of its body and builds at \
         line {compiles}. A copy taken after the build is a copy of the rewrite."
    );
    assert!(
        body.contains("cmp -s \"$before\" \"$lock\""),
        "`just {REWRITE_GATE}` never compares {FUZZ_LOCK} against the copy it took, so the copy \
         decides nothing"
    );
}

#[test]
fn the_machinery_this_exemption_promises_is_machinery_a_person_can_run() {
    // The test above asks whether the two lockfiles agree. This one asks what
    // a person does when they do not, and until this PR the honest answer was
    // "work it out": the exemption named two gates and neither of them, nor
    // the assertion above, named a command. The only thing in the repo that
    // re-resolved that file was `just fuzz-build` — nightly, four libFuzzer
    // targets, and an exit 1 by construction once it had done it. A repair
    // reachable only through a gate that fails on success is how a drifting
    // lockfile gets committed drifted.
    //
    // So the promise is checked as a promise: every recipe the exemption names
    // exists, and the repair the failures name exists, re-resolves the right
    // lockfile, and is spelled in every message that reports the drift.
    let justfile = fs::read_to_string(repo_root().join("Justfile"))
        .unwrap_or_else(|e| panic!("reading Justfile: {e}"));

    let (_, reason) = CARGO_SELF_LOCKING
        .iter()
        .find(|(sub, _)| *sub == MACHINERY_EXEMPTION)
        .unwrap_or_else(|| {
            panic!("`{MACHINERY_EXEMPTION}` is no longer an exemption; this reader is stale")
        });
    let promised = recipes_named_in(reason);
    assert!(
        promised.len() >= 2,
        "the `{MACHINERY_EXEMPTION}` exemption came out naming {promised:?}. Its reason is the \
         one on this list that promises machinery instead of describing a CLI, and a reader that \
         finds no machinery in it blesses whatever the sentence says next."
    );
    for name in &promised {
        assert!(
            recipe(&justfile, name).is_some(),
            "the `{MACHINERY_EXEMPTION}` exemption rests on `just {name}`, which this Justfile \
             does not define. The exemption is then a sentence about a command nobody can run, \
             and `cargo fuzz` is unbound with nothing standing in for the flag."
        );
    }

    let repair = recipe(&justfile, FUZZ_LOCK_REPAIR).unwrap_or_else(|| {
        panic!(
            "`just {FUZZ_LOCK_REPAIR}` is not a recipe. Two gate failures and the assertion above \
             tell a person to run it, so a rename here costs three messages at once — each of \
             them still perfectly clear, and none of them true."
        )
    });
    let body = repair.body.join("\n");

    let manifest = FUZZ_LOCK.replace("Cargo.lock", "Cargo.toml");
    assert!(
        body.contains(&manifest),
        "`just {FUZZ_LOCK_REPAIR}` never names {manifest}. Re-resolving the OTHER workspace is \
         what it is for; a cargo run that does not name that manifest rewrites this workspace's \
         lockfile instead, which is the file that was already right."
    );

    let subcommands = cargo_subcommands(&body);
    assert!(
        !subcommands.is_empty(),
        "`just {FUZZ_LOCK_REPAIR}` spells no cargo call: {body}"
    );
    for sub in &subcommands {
        assert!(
            CARGO_SELF_LOCKING.iter().any(|&(name, _)| name == *sub),
            "`just {FUZZ_LOCK_REPAIR}` re-resolves with `cargo {sub}`, which this file's own \
             policy binds to a lockfile with `--locked`. A repair that may not rewrite the file \
             repairs nothing — the sub-command has to be one whose job IS the rewrite."
        );
    }

    assert!(
        !repair.attributes.iter().any(|line| line.contains("'gate'")),
        "`just {FUZZ_LOCK_REPAIR}` is tagged a gate, so `just ci` runs it. It rewrites the very \
         file the two gates below compare, and it runs before them: the drift would be repaired \
         in the working tree and reported by nobody, leaving two green gates over an uncommitted \
         diff. The repair is a thing a person runs."
    );

    for &(gate, compares) in GATES_THAT_REPORT_THIS_DRIFT {
        let reporter = recipe(&justfile, gate)
            .unwrap_or_else(|| panic!("`just {gate}` ({compares}) is not a recipe any more"));
        assert!(
            reporter
                .attributes
                .iter()
                .any(|line| line.contains("'gate'")),
            "`just {gate}` is no longer a gate, so nothing runs it and the drift it {compares} \
             is reported on no pull request"
        );
        assert!(
            recipes_named_in(&reporter.body.join("\n")).contains(FUZZ_LOCK_REPAIR),
            "`just {gate}` {compares} and its failure never says `just {FUZZ_LOCK_REPAIR}`. \
             This drift arrives on every cargo bump that moves a version the fuzz graph reaches, \
             and the person reading the failure is usually meeting the second lockfile for the \
             first time."
        );
    }
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

/// The one entry on [`CARGO_SELF_LOCKING`] whose reason names no substitute,
/// because there is none.
const UNPINNED_BASELINE: &str = "semver-checks";

#[test]
fn the_exemption_nothing_stands_in_for_is_measured_rather_than_asserted() {
    // Every other reason on that list is a fact about this repo, and something
    // here holds it: `fmt` and `audit` cannot resolve a different graph at all,
    // and the `fuzz` entry names a committed lockfile plus the two gates that
    // watch it. This one is a fact about somebody else's CLI. The `semver`
    // gate builds the baseline revision in a worktree of its own and that
    // build resolves its own graph, so which graph proved the comparison is
    // the one resolution in this repo that is not pinned (DEV-298) — and the
    // whole justification for accepting that is a sentence about a flag not
    // existing, written once, about a tool that ships releases.
    let (_, reason) = CARGO_SELF_LOCKING
        .iter()
        .find(|(sub, _)| *sub == UNPINNED_BASELINE)
        .unwrap_or_else(|| {
            panic!("`{UNPINNED_BASELINE}` is no longer an exemption; this reader is stale")
        });
    assert!(
        reason.contains("--locked"),
        "the exemption for `{UNPINNED_BASELINE}` no longer claims the flag is missing: {reason}"
    );

    let help = Spawn::new("cargo")
        .args([UNPINNED_BASELINE, "check-release", "--help"])
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running cargo {UNPINNED_BASELINE}: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where it is installed."
            )
        });
    let usage = String::from_utf8_lossy(&help.stdout);
    assert!(
        usage.contains("--baseline-rev"),
        "cargo {UNPINNED_BASELINE} printed no usage this reader recognises:\n{usage}"
    );
    assert!(
        !usage.contains("--locked"),
        "cargo {UNPINNED_BASELINE} now takes `--locked`. The exemption was true when it was \
         written and is not any more: move `{UNPINNED_BASELINE}` to CARGO_RESOLVING, pass the \
         flag in the `semver` recipe, and close DEV-298 — the baseline build has stopped being \
         the one resolution here that nothing pins."
    );
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
fn a_lockfile_reads_as_its_packages_and_the_format_version_is_not_one() {
    // Every `[[package]]` block is name-then-version, and the file opens with a
    // bare `version = 4` that belongs to the format rather than to a crate. A
    // reader that took that one would attribute it to whichever package it had
    // seen last — or, first thing in the file, to none, and then quietly report
    // one package fewer than the lockfile holds.
    let lock = concat!(
        "version = 4\n",
        "\n",
        "[[package]]\n",
        "name = \"comrak\"\n",
        "version = \"0.52.0\"\n",
        "dependencies = [\n",
        " \"entities\",\n",
        "]\n",
        "\n",
        "[[package]]\n",
        "name = \"rustc-hash\"\n",
        "version = \"1.1.0\"\n",
        "\n",
        "[[package]]\n",
        "name = \"rustc-hash\"\n",
        "version = \"2.1.3\"\n",
    );
    let read_here = locked_versions(lock);
    assert_eq!(
        read_here.len(),
        2,
        "the format version was read as a package, or a block was missed: {read_here:?}"
    );
    assert_eq!(
        read_here["comrak"],
        BTreeSet::from(["0.52.0".to_owned()]),
        "a package's version went unread: {read_here:?}"
    );
    // Two majors of one crate, which is why the comparison is over sets: the
    // workspace really does hold two of `rustc-hash` and two of
    // `unicode-width`, and a reader keeping one version per name would call the
    // fuzz lockfile drifted for resolving the other.
    assert_eq!(
        read_here["rustc-hash"],
        BTreeSet::from(["1.1.0".to_owned(), "2.1.3".to_owned()]),
        "a second version of one package overwrote the first: {read_here:?}"
    );
    assert!(
        BTreeSet::from(["2.1.3".to_owned()]).is_subset(&read_here["rustc-hash"]),
        "the subset relation the assertion rests on does not hold for a package resolved twice"
    );
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
