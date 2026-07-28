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
        "cargo-fuzz 0.13.2 has no `--locked` and no way to pass one to the \
         `cargo build` it shells out to. The fuzz crate's `Cargo.lock` is \
         committed instead (DEV-293), `just fuzz-build` fails when a build \
         rewrote it — the same refusal, one step later — and \
         `just verify-version-pins` compares the versions the two lockfiles \
         resolve to",
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

    let fuzz = locked_versions(&fuzz);
    let workspace = locked_versions(&workspace);
    let shared: Vec<&String> = fuzz
        .keys()
        .filter(|name| workspace.contains_key(*name))
        .collect();
    assert!(
        shared.len() >= 40 && workspace.len() >= 100,
        "{} packages in the fuzz lockfile, {} in the workspace's, {} shared; the reader is not \
         finding the `[[package]]` blocks",
        fuzz.len(),
        workspace.len(),
        shared.len()
    );

    let mut drifted = Vec::new();
    for name in shared {
        let (theirs, ours) = (&fuzz[name], &workspace[name]);
        if !theirs.is_subset(ours) {
            drifted.push(format!("  {name}: fuzz {theirs:?} vs workspace {ours:?}"));
        }
    }
    assert!(
        drifted.is_empty(),
        "the fuzz targets and the library resolve different versions of the same crates:\n{}\n\
         A subset is fine — the fuzz graph is the smaller one, and the workspace keeps two majors \
         of a few crates for dependencies the harnesses never reach. A version the workspace does \
         not have is not: the targets are then fuzzing code this repo does not ship. Re-resolve \
         the fuzz workspace and commit {FUZZ_LOCK}.",
        drifted.join("\n")
    );

    // And that the two names the recipe does compare are among them, so its
    // narrower check is narrow rather than vacuous.
    for name in ["aozora", "comrak"] {
        assert!(
            fuzz.contains_key(name) && workspace.contains_key(name),
            "`{name}` is not in both lockfiles, and `just verify-version-pins` compares exactly \
             it and one other by name — over a package one side does not have, that comparison \
             passes on two empty strings"
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
