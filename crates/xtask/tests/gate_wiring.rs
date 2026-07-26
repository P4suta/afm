//! A gate this repo declares is a gate this repo runs.
//!
//! Two ways a check can exist and check nothing, both of which have already
//! happened here. `actionlint` sat in `mise.toml` from the start and was
//! invoked by nothing — a tool the repo names, pins and never calls reads to
//! every newcomer as a gate that is running. And `just ci` once drifted out of
//! step with the CI job set (it grew `playground-build` afterwards), so the
//! command whose whole promise is "exactly the gate CI runs" was quietly a
//! subset of it.
//!
//! Both are one failure: the wiring between declaring a check and executing it
//! is spread over `mise.toml`, the `Justfile`, `lefthook.yml` and `ci.yml`,
//! and no compiler compares those. This does.

#![forbid(unsafe_code)]

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// reading a command out of a config file
// ---------------------------------------------------------------------------

/// Drop a `#` comment. Only a `#` opening a word is one, so a `#` inside an
/// argument (`grep -q "#${pin}"`) survives.
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

/// The words a shell would see, splitting on everything a command name cannot
/// contain.
fn words(line: &str) -> impl Iterator<Item = &str> {
    line.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_'))
        .filter(|word| !word.is_empty())
}

/// Every word appearing where this repo executes things: a `Justfile` recipe
/// *body* (indented — a recipe header names a dependency, it does not run a
/// binary) and a `lefthook.yml` `run:`.
///
/// Comments come off first, and that is the point. The `Justfile` discusses
/// `zizmor` and `actionlint` at length in prose, and prose counting as
/// enforcement is the exact thing this file exists to reject.
fn executed_words(justfile: &str, lefthook: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in justfile.lines() {
        if !line.starts_with([' ', '\t']) {
            continue;
        }
        out.extend(words(strip_comment(line)).map(str::to_owned));
    }
    for line in lefthook.lines() {
        if let Some(command) = line.trim_start().strip_prefix("run:") {
            out.extend(words(command).map(str::to_owned));
        }
    }
    out
}

/// The tool names in `mise.toml`'s `[tools]` table. A key may be a backend
/// path (`github:crate-ci/typos`); the command is its last segment.
fn declared_tools(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut in_tools = false;
    for line in text.lines() {
        let body = strip_comment(line).trim();
        if body.starts_with('[') {
            in_tools = body == "[tools]";
            continue;
        }
        let Some((key, _)) = body.split_once('=').filter(|_| in_tools) else {
            continue;
        };
        let key = key.trim().trim_matches('"');
        out.push(key.rsplit('/').next().unwrap_or(key).to_owned());
    }
    out
}

/// What a `Justfile` recipe depends on, off its header line.
fn recipe_dependencies(justfile: &str, recipe: &str) -> Vec<String> {
    let header = justfile
        .lines()
        .find(|line| line.starts_with(&format!("{recipe}:")))
        .unwrap_or_else(|| panic!("the Justfile has no `{recipe}:` recipe any more"));
    let (_, deps) = header.split_once(':').expect("found by its colon");
    words(strip_comment(deps)).map(str::to_owned).collect()
}

/// The steps `just ci` runs, out of the two bash arrays that drive it.
fn ci_recipe_steps(justfile: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for array in ["fg_steps=(", "bg_steps=("] {
        let after = justfile
            .split_once(array)
            .unwrap_or_else(|| panic!("the `ci` recipe no longer builds `{array}…)`"))
            .1;
        let body = after
            .split_once(')')
            .unwrap_or_else(|| panic!("`{array}` is never closed"))
            .0;
        out.extend(words(body).map(str::to_owned));
    }
    out
}

/// The lines of one top-level job in a workflow, by its key.
fn job_lines(workflow: &str, job: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in workflow.lines() {
        let body = line.trim();
        let is_job_key = line.len() - body.len() == 2 && !body.is_empty() && !body.starts_with('#');
        if inside && is_job_key {
            break;
        }
        if is_job_key && body == format!("{job}:") {
            inside = true;
            continue;
        }
        if inside {
            out.push(line.to_owned());
        }
    }
    assert!(!out.is_empty(), "the workflow has no `{job}:` job any more");
    out
}

/// The legs of a workflow job's `matrix.target` list.
fn matrix_targets(workflow: &str, job: &str) -> BTreeSet<String> {
    let lines = job_lines(workflow, job);
    let list = lines
        .iter()
        .find_map(|line| line.trim().strip_prefix("target:"))
        .unwrap_or_else(|| panic!("the `{job}` job no longer matrixes on `target`"))
        .to_owned();
    words(&list).map(str::to_owned).collect()
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

fn read(relative: &str) -> String {
    let path = repo_root().join(relative);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
}

// ---------------------------------------------------------------------------
// the invariants
// ---------------------------------------------------------------------------

#[test]
fn every_tool_this_repo_declares_is_a_tool_this_repo_runs() {
    let declared = declared_tools(&read("mise.toml"));
    assert!(
        declared.len() >= 5,
        "mise.toml yielded {} tools; the reader is not finding the `[tools]` table",
        declared.len()
    );

    let executed = executed_words(&read("Justfile"), &read("lefthook.yml"));
    let idle: Vec<&String> = declared
        .iter()
        .filter(|tool| !executed.contains(*tool))
        .collect();
    assert!(
        idle.is_empty(),
        "declared in mise.toml and invoked nowhere: {idle:?}\n\
         A pinned, installed tool nothing calls reads as a gate and is not one. \
         Give it a `just` recipe (and a place in `just lint` / `just ci`), or drop \
         the declaration."
    );
}

#[test]
fn every_lint_gate_runs_in_ci_and_every_ci_lint_leg_is_a_gate() {
    // `just lint` is what a developer runs; ci.yml's `lint` matrix is what
    // merges the PR. A recipe in one and not the other is either a check
    // nothing enforces or a check nobody can reproduce.
    let local: BTreeSet<String> = recipe_dependencies(&read("Justfile"), "lint")
        .into_iter()
        .collect();
    let remote = matrix_targets(&read(".github/workflows/ci.yml"), "lint");

    assert!(
        local.len() >= 5,
        "`just lint` came out as {local:?}; the reader is not finding its header"
    );
    assert_eq!(
        local,
        remote,
        "`just lint` and ci.yml's `lint` matrix have drifted.\n  \
         only local: {:?}\n  only in CI: {:?}",
        local.difference(&remote).collect::<Vec<_>>(),
        remote.difference(&local).collect::<Vec<_>>()
    );
}

#[test]
fn every_ci_lint_leg_is_also_a_step_of_just_ci() {
    // `just ci` calls itself "exactly the gate CI runs". The way that sentence
    // stops being true is a leg added to the workflow matrix and not to the
    // recipe's step list — which is how `playground-build` went missing once.
    let remote = matrix_targets(&read(".github/workflows/ci.yml"), "lint");
    let steps = ci_recipe_steps(&read("Justfile"));
    let missing: Vec<&String> = remote.iter().filter(|leg| !steps.contains(*leg)).collect();
    assert!(
        missing.is_empty(),
        "ci.yml lints these and `just ci` does not run them: {missing:?}\n\
         `just ci` has to be a superset of CI, or it cannot be the thing you run \
         before pushing."
    );
}

// ---------------------------------------------------------------------------
// what the readers claim, pinned both ways
// ---------------------------------------------------------------------------

#[test]
fn the_workflow_linters_are_wired_and_not_merely_available() {
    // The failure this file is about, stated as a fact rather than a property:
    // `actionlint` was declared in mise.toml and called nowhere.
    let executed = executed_words(&read("Justfile"), &read("lefthook.yml"));
    for tool in ["zizmor", "actionlint", "typos", "lefthook", "committed"] {
        assert!(executed.contains(tool), "`{tool}` is invoked nowhere");
    }
}

#[test]
fn a_tool_named_only_in_prose_does_not_count_as_invoked() {
    // Prose describing a rule nobody runs is what `ci.yml`'s old header did
    // about pinning. If comments counted here, this test would pass on the
    // very state it exists to reject.
    let justfile = concat!(
        "# Run all lints, zizmor and actionlint included\n",
        "lint: fmt-check actionlint\n",
        "    # remember to run zizmor before pushing\n",
        "    typos --format brief\n",
    );
    let executed = executed_words(justfile, "");
    assert!(
        executed.contains("typos"),
        "an invocation in a recipe body was missed: {executed:?}"
    );
    assert!(
        !executed.contains("zizmor"),
        "a tool named only in a comment counted as invoked"
    );
    // A header names recipes; it does not execute binaries. Were column-zero
    // lines counted, a recipe named after a tool would vouch for a tool it
    // never runs.
    assert!(
        !executed.contains("actionlint"),
        "a recipe header counted as an invocation"
    );
}

#[test]
fn a_hash_inside_an_argument_does_not_truncate_the_command() {
    let kept: Vec<&str> = words(strip_comment(r##"    grep -q "#${pin}" Cargo.lock"##)).collect();
    assert!(
        kept.contains(&"Cargo"),
        "comment stripping swallowed the rest of a command: {kept:?}"
    );
}

#[test]
fn a_backend_prefixed_tool_declaration_reads_as_its_command_name() {
    let mise = concat!(
        "[tools]\n",
        "just = \"latest\"\n",
        "\"github:crate-ci/typos\" = \"latest\"\n",
        "\n",
        "[env]\n",
        "NOT_A_TOOL = \"1\"\n",
    );
    assert_eq!(
        declared_tools(mise),
        vec!["just".to_owned(), "typos".to_owned()],
        "the `[tools]` reader picked up the wrong table or the wrong name"
    );
}
