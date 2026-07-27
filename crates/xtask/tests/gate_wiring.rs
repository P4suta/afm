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
//!
//! One side of it has since been closed by construction rather than by
//! comparison: `[group('gate')]` is the only declaration of what a gate is,
//! ci.yml generates its matrix from that attribute, and so a leg cannot exist
//! in the workflow that does not exist in the `Justfile`. What is left to
//! check is the two lists this repo still writes by hand — the recipes
//! `just lint` bundles, and the lanes inside the `ci` recipe.
//!
//! Derivation moves the drift rather than ending it, so the rest of this file
//! guards the seams it created. The attribute is now read twice — by `just`
//! at run time and by the reader below — and a workflow that stopped
//! expanding the manifest, a `[group('native')]` gate with no job to run it,
//! or a gate whose recipe cannot be invoked by name are all states where the
//! declaration is still perfectly consistent and something has quietly
//! stopped running.
//!
//! There is a third way, and the `doc` gate was in it: a check that is
//! declared, is run, and passes on the defect it exists for. rustdoc's lints
//! are warn-by-default bar one, so `cargo doc` reports and exits 0; what made
//! the gate bite at all was `[workspace.lints.rustdoc]`, which is a
//! hand-written list of eight lints that reaches a crate only if that crate's
//! manifest opts in with `[lints] workspace = true`. Neither half covers a
//! lint nobody listed or a crate nobody opted in — and the `-D warnings` that
//! does was written in `docs.yml`, i.e. one Pages deploy after the merge.
//!
//! Nothing above notices any of that, because every assertion up to here is
//! about a recipe's NAME. The middle sections read what a gate hands its tool,
//! hold the rustdoc build the repo publishes to the shape docs.rs will
//! build, and run rustdoc against a probe so that "these flags deny" is
//! measured rather than spelled.
//!
//! A fourth way, and `just audit` was in it: a check that is declared, is run,
//! is strict, and is only ever asked at the wrong moment. Every rule above
//! holds a gate to the tree it is handed, which is the right frame for all of
//! them but two. The advisory scans are functions of a database elsewhere, so
//! a run triggered by a diff answers whether THIS diff added a finding and
//! nothing answers whether one appeared against a lockfile nobody edited —
//! which is how advisories normally appear. `SECURITY.md` stated that gap as
//! policy ("there is no cron workflow"), which is the same defect written
//! where a reader would mistake it for a decision. The last section is about
//! when a gate is asked, and about what a scheduled answer reaches.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU32, Ordering};

use regex::Regex;
use serde_json::Value;

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

/// The same split with `.` kept inside a word, for reading a workflow rather
/// than a command line. ci.yml's paths filter lists `clippy.toml` and
/// `deny.toml`; a scanner that split those would read the file as naming the
/// `clippy` and `deny` gates, which is the one thing it is asked to detect.
fn dotted_words(line: &str) -> impl Iterator<Item = &str> {
    line.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '.'))
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
    header_dependencies(recipe_header(justfile, recipe))
}

/// The same question asked of a header already in hand. Found by name rather
/// than by prefix, because a recipe may take parameters: the dependencies of
/// `test *ARGS:` are not on a line that starts `test:`.
fn header_dependencies(header: &str) -> Vec<String> {
    let (_, deps) = header.split_once(':').unwrap_or((header, ""));
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

/// The name off a recipe header, parameters and dependencies dropped. `None`
/// for a `:=` assignment, which is the other thing that starts at column zero.
fn recipe_name(header: &str) -> Option<String> {
    let (name, after) = header.split_once(':')?;
    if after.starts_with('=') {
        return None;
    }
    Some(name.split_whitespace().next()?.to_owned())
}

/// The groups one attribute line declares. `just` accepts several attributes
/// on a line and either quote around the name, so `[group('gate'),
/// group("native")]` declares two — verified against the pinned 1.51.0 and
/// against latest. Matching the whole line instead would read that as neither,
/// and a gate the tests cannot see is worse than one they reject.
fn attribute_groups(line: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some((_, after)) = rest.split_once("group(") {
        let Some((name, tail)) = after.split_once(')') else {
            break;
        };
        out.push(name.trim().trim_matches(['\'', '"']));
        rest = tail;
    }
    out
}

/// The recipes carrying `[group('<group>')]`. This attribute is the manifest
/// the whole gate set is derived from: `just gates` reads it at runtime, and
/// ci.yml expands what that prints into its job matrix.
fn recipes_in_group(justfile: &str, group: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut tagged = false;
    for line in justfile.lines() {
        if line.starts_with('[') {
            tagged |= attribute_groups(line).contains(&group);
            continue;
        }
        if line.starts_with('#') || line.starts_with([' ', '\t']) || line.trim().is_empty() {
            continue;
        }
        if tagged {
            out.extend(recipe_name(line));
        }
        tagged = false;
    }
    out
}

/// The header line that defines a recipe, found by name rather than by
/// prefix — a gate may take parameters, so `commitlint RANGE="…":` is the
/// header of `commitlint`.
fn recipe_header<'a>(justfile: &'a str, recipe: &str) -> &'a str {
    justfile
        .lines()
        .find(|line| {
            !line.starts_with([' ', '\t', '#', '[']) && recipe_name(line).as_deref() == Some(recipe)
        })
        .unwrap_or_else(|| panic!("the Justfile has no `{recipe}` recipe any more"))
}

/// The parameters a recipe header declares, as written: `RANGE="a..b"`,
/// `*ARGS`, `SEED`.
fn recipe_parameters(header: &str) -> Vec<&str> {
    let signature = header.split_once(':').map_or(header, |(before, _)| before);
    signature.split_whitespace().skip(1).collect()
}

/// The body of the `ci` recipe, in order. Order is the point: what the recipe
/// checks before it dispatches anything is a different promise from what it
/// checks eventually.
fn ci_recipe_body(justfile: &str) -> Vec<&str> {
    justfile
        .lines()
        .skip_while(|line| !line.starts_with("ci:"))
        .skip(1)
        .take_while(|line| line.starts_with([' ', '\t']) || line.trim().is_empty())
        .collect()
}

/// Does this line run `just <recipe>`?
fn line_runs_recipe(line: &str, recipe: &str) -> bool {
    let mut previous = "";
    for word in dotted_words(strip_comment(line)) {
        if previous == "just" && word == recipe {
            return true;
        }
        previous = word;
    }
    false
}

fn runs_recipe(lines: &[&str], recipe: &str) -> bool {
    lines.iter().any(|line| line_runs_recipe(line, recipe))
}

// ---------------------------------------------------------------------------
// reading the workflow
// ---------------------------------------------------------------------------

/// The lines under one top-level mapping key. `jobs:`, `on:`, `env:` and
/// `permissions:` all hold keys indented alike, so only the column-zero header
/// above them says which block a line is in.
fn top_level_block<'a>(workflow: &'a str, key: &str) -> Vec<&'a str> {
    let header = format!("{key}:");
    let mut out = Vec::new();
    let mut inside = false;
    for line in workflow.lines() {
        if line.starts_with([' ', '\t']) || line.trim().is_empty() {
            if inside {
                out.push(line);
            }
            continue;
        }
        // A column-zero comment interrupts nothing; any other column-zero line
        // is the next top-level key.
        if !line.starts_with('#') {
            inside = line.trim_end() == header;
        }
    }
    out
}

/// The lines under the workflow's `jobs:` mapping.
fn jobs_block(workflow: &str) -> Vec<&str> {
    top_level_block(workflow, "jobs")
}

/// The job a line opens, if it opens one.
fn job_key(line: &str) -> Option<&str> {
    let body = line.strip_prefix("  ")?;
    if body.starts_with([' ', '\t', '#']) {
        return None;
    }
    let key = body.trim_end().strip_suffix(':')?;
    (!key.contains(char::is_whitespace)).then_some(key)
}

fn job_keys(workflow: &str) -> BTreeSet<String> {
    jobs_block(workflow)
        .into_iter()
        .filter_map(job_key)
        .map(str::to_owned)
        .collect()
}

/// The lines of one job, or `None` when the workflow has no such job. A
/// missing job is a state this file has to report on, not panic over: it is
/// exactly what "the gate lost its runner" looks like.
fn job_lines<'a>(workflow: &'a str, job: &str) -> Option<Vec<&'a str>> {
    let mut out = Vec::new();
    let mut inside = false;
    for line in jobs_block(workflow) {
        if let Some(key) = job_key(line) {
            if inside {
                break;
            }
            inside = key == job;
            continue;
        }
        if inside {
            out.push(line);
        }
    }
    inside.then_some(out)
}

/// The jobs one job waits on, in either spelling `needs:` takes.
fn job_needs(workflow: &str, job: &str) -> BTreeSet<String> {
    let lines = job_lines(workflow, job)
        .unwrap_or_else(|| panic!("the workflow has no `{job}:` job any more"));
    let mut out = BTreeSet::new();
    let mut collecting = false;
    for line in lines {
        let body = strip_comment(line).trim();
        if let Some(inline) = body.strip_prefix("needs:") {
            if !inline.trim().is_empty() {
                return words(inline).map(str::to_owned).collect();
            }
            collecting = true;
            continue;
        }
        if !collecting || body.is_empty() {
            continue;
        }
        let Some(item) = body.strip_prefix("- ") else {
            break;
        };
        out.insert(item.trim().to_owned());
    }
    out
}

/// The recipes a workflow spells out: the argument after each `just`. An
/// argument that is an expression (`just "$GATE"`) is left out — that one is
/// the manifest, which is the point of it.
fn recipes_invoked(workflow: &str) -> BTreeSet<String> {
    recipes_invoked_in(&jobs_block(workflow))
}

/// The same question asked of one job's lines rather than a whole workflow.
/// Which job runs a recipe is what decides whether it needs to be told where
/// it is, so the answer has to be per-job.
fn recipes_invoked_in(lines: &[&str]) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in lines {
        let mut rest = strip_comment(line);
        while let Some((_, after)) = rest.split_once("just ") {
            rest = after;
            let argument = after
                .split_whitespace()
                .next()
                .unwrap_or_default()
                .trim_start_matches(['"', '\'']);
            if argument.starts_with('$') {
                continue;
            }
            // Cut at the first character a recipe name cannot hold, so the
            // shell around the call — `<(just gates)`, `just doc;` — does not
            // arrive as part of it.
            let name = words(argument).next().unwrap_or_default();
            if !name.is_empty() {
                out.insert(name.to_owned());
            }
        }
    }
    out
}

/// Every word the `jobs:` mapping actually says, comments off. What ci.yml
/// discusses in prose is not what it runs — the header still names `msrv`,
/// `commitlint` and `prop` to explain the drift they used to be.
fn workflow_vocabulary(workflow: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for line in jobs_block(workflow) {
        out.extend(dotted_words(strip_comment(line)).map(str::to_owned));
    }
    out
}

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// What `just gates <group>` prints: the manifest as ci.yml expands it and as
/// `just ci` asserts against it. Shelling out is the point — every other
/// assertion here reads the attribute with the reader above, and the two
/// answers are only known to agree if something asks both.
fn just_gates(group: &str) -> BTreeSet<String> {
    let out = Command::new("just")
        .args(["gates", group])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running `just gates {group}`: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where `just` is installed."
            )
        });
    assert!(
        out.status.success(),
        "`just gates {group}` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
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
fn every_lint_the_bundle_runs_is_a_gate_the_manifest_declares() {
    // `just lint` is the bundle a developer reaches for; `[group('gate')]` is
    // what puts a recipe in CI. A lint in the bundle and not in the manifest
    // is a check that passes locally and merges the PR without ever running.
    let justfile = read("Justfile");
    let bundled: BTreeSet<String> = recipe_dependencies(&justfile, "lint").into_iter().collect();
    let manifest = recipes_in_group(&justfile, "gate");

    assert!(
        bundled.len() >= 5,
        "`just lint` came out as {bundled:?}; the reader is not finding its header"
    );
    let ungated: Vec<&String> = bundled.difference(&manifest).collect();
    assert!(
        ungated.is_empty(),
        "`just lint` bundles these and no `[group('gate')]` declares them: {ungated:?}\n\
         Tag the recipe, or take it out of the bundle — a lint nothing gates is \
         one nobody has to pass."
    );
}

#[test]
fn the_gate_manifest_is_exactly_what_just_ci_runs() {
    // `just ci` calls itself "exactly the gate CI runs". CI's half is now
    // generated from `[group('gate')]`, so what is left to compare is the
    // recipe's own two lanes against the same manifest. `just ci` asserts this
    // at run time too; here it is decidable from the file, so a mismatch costs
    // a test rather than a pipeline.
    let justfile = read("Justfile");
    let manifest = recipes_in_group(&justfile, "gate");
    let lanes = ci_recipe_steps(&justfile);

    assert!(
        manifest.len() >= 15,
        "`[group('gate')]` came out as {manifest:?}; the reader is not finding the attribute"
    );
    assert_eq!(
        manifest,
        lanes,
        "the gate manifest and `just ci`'s lanes have drifted.\n  \
         tagged, not run: {:?}\n  run, not tagged: {:?}",
        manifest.difference(&lanes).collect::<Vec<_>>(),
        lanes.difference(&manifest).collect::<Vec<_>>()
    );
}

#[test]
fn the_manifest_the_reader_sees_is_the_manifest_just_prints() {
    // Every other assertion in this file reads `[group('gate')]` with the
    // reader above. CI runs the legs `just gates` prints. Nothing compared
    // those, and they can disagree while both look right: `[private]` on a
    // gate recipe drops it from `just --list` — hence from the matrix AND
    // from `just ci`'s own runtime assert, which reads the same printout —
    // while the attribute it was tagged with is still sitting there in the
    // file for a reader (or a reviewer) to count.
    let justfile = read("Justfile");
    for group in ["gate", "native"] {
        let read_here = recipes_in_group(&justfile, group);
        assert!(
            !read_here.is_empty(),
            "`[group('{group}')]` came out empty; the reader is not finding the attribute"
        );
        assert_eq!(
            read_here,
            just_gates(group),
            "`[group('{group}')]` as this file reads it is not what `just gates {group}` prints. \
             CI expands the printout and these tests read the attribute, so a recipe in one and \
             not the other is a gate that either runs unexamined or is examined and never runs."
        );
    }
}

#[test]
fn every_native_gate_has_a_job_that_runs_the_same_recipe() {
    // `[group('native')]` takes a gate OUT of the matrix, on the promise of a
    // hand-written job that runs the same recipe against a toolchain the dev
    // image does not have. Nothing enforced the second half: tagging a recipe
    // `native` and stopping there deletes it from CI in one line, silently and
    // with the manifest still perfectly consistent.
    let justfile = read("Justfile");
    let workflow = read(".github/workflows/ci.yml");
    let manifest = recipes_in_group(&justfile, "gate");
    let native = recipes_in_group(&justfile, "native");

    assert!(
        !native.is_empty(),
        "`[group('native')]` came out empty; the reader is not finding the attribute"
    );
    for gate in &native {
        assert!(
            manifest.contains(gate),
            "`{gate}` is `[group('native')]` without `[group('gate')]`: out of the matrix by the \
             first attribute and out of the manifest by the missing second, so nothing runs it."
        );
        let job = job_lines(&workflow, gate).unwrap_or_else(|| {
            panic!(
                "`{gate}` is tagged `[group('native')]`, which excludes it from the gate matrix, \
                 and ci.yml has no `{gate}:` job to run it instead."
            )
        });
        assert!(
            runs_recipe(&job, gate),
            "ci.yml's `{gate}` job does not run `just {gate}`. A native gate that spells its \
             command out again is a second definition of the check — which is how the msrv job \
             came to run a `cargo check` that `just ci` never ran."
        );
        assert!(
            job.iter().any(|line| strip_comment(line)
                .trim()
                .starts_with("AOZORA_MD_IN_CONTAINER:")),
            "ci.yml's `{gate}` job does not set AOZORA_MD_IN_CONTAINER. Without it the recipe's \
             `_in` switch wraps the command in `docker compose run`, so the job would test the \
             dev image's toolchain — not the one it installed, which is its whole reason to exist."
        );
    }
}

#[test]
fn the_workflow_hand_writes_no_gate_of_its_own() {
    // The old shape of this file compared ci.yml's hand-written lint matrix
    // against `just lint`. The matrix is generated now, so that comparison is
    // gone — and with it the only thing that would notice a leg being written
    // back in by hand. A gate named in ci.yml is a gate the manifest does not
    // control: it can be a job, a matrix leg, or a `- run: just <name>` step,
    // and all three are the drift this PR removed.
    let justfile = read("Justfile");
    let workflow = read(".github/workflows/ci.yml");
    let native = recipes_in_group(&justfile, "native");
    let containerized: BTreeSet<String> = recipes_in_group(&justfile, "gate")
        .difference(&native)
        .cloned()
        .collect();
    assert!(
        containerized.len() >= 15,
        "the matrix legs came out as {containerized:?}; the reader is not finding the attribute"
    );

    let vocabulary = workflow_vocabulary(&workflow);
    let hand_written: Vec<&String> = containerized
        .iter()
        .filter(|gate| vocabulary.contains(*gate))
        .collect();
    assert!(
        hand_written.is_empty(),
        "ci.yml names these gates itself: {hand_written:?}\n\
         The matrix comes from `just gates`; spelling a gate out here gives it a second, \
         hand-maintained definition — the exact arrangement that let `prop` run locally and \
         nowhere else. Only `[group('native')]` gates get a job of their own."
    );
}

/// The recipes ci.yml may run without being gates, each with the reason it is
/// not one. Anything else it runs is a check `just ci` does not run — the
/// original defect, in the direction the manifest does not close by itself.
const NOT_A_GATE_IN_CI: &[(&str, &str)] = &[
    (
        "check",
        "a scheduling precondition: one cheap compile the matrix waits on, so a \
         compile error costs one runner. `build` and `clippy` both compile \
         --all-targets, so it gates nothing they do not.",
    ),
    (
        "gates",
        "the manifest query itself — the thing that decides what the gates are \
         cannot be one of them.",
    ),
];

#[test]
fn every_recipe_the_workflow_runs_is_a_gate_or_a_named_precondition() {
    // The other direction of the same drift, and the one derivation does not
    // close: nothing stops a `- run: just <something>` step being added back
    // to ci.yml. That is how `wasm-build` came to run in CI and not in
    // `just ci` — a check the workflow enforces that no local command
    // reproduces is exactly as broken as a check that runs nowhere.
    let justfile = read("Justfile");
    let workflow = read(".github/workflows/ci.yml");
    let manifest = recipes_in_group(&justfile, "gate");
    let invoked = recipes_invoked(&workflow);

    assert!(
        invoked.len() >= 3,
        "ci.yml came out running {invoked:?}; the reader is not finding its `just` steps"
    );
    let unaccounted: Vec<&String> = invoked
        .iter()
        .filter(|recipe| {
            !manifest.contains(*recipe)
                && !NOT_A_GATE_IN_CI
                    .iter()
                    .any(|(allowed, _)| allowed == recipe)
        })
        .collect();
    assert!(
        unaccounted.is_empty(),
        "ci.yml runs these and `[group('gate')]` does not declare them: {unaccounted:?}\n\
         Tag the recipe so `just ci` runs it too, or add it to NOT_A_GATE_IN_CI with the reason \
         it is not a gate."
    );
}

#[test]
fn the_gate_matrix_is_expanded_from_the_manifest_job() {
    // "Add a gate and it appears in CI" holds only while the legs are the
    // manifest. A literal list here would still pass every other test in this
    // file, because a workflow that agrees with the Justfile today is exactly
    // what a workflow that has stopped reading it looks like on day one.
    let workflow = read(".github/workflows/ci.yml");

    let matrix = job_lines(&workflow, "gate")
        .unwrap_or_else(|| panic!("ci.yml has no `gate:` job — the legs are hand-written again"));
    let legs = matrix
        .iter()
        .map(|line| strip_comment(line).trim())
        .find(|line| line.starts_with("gate:"))
        .unwrap_or_else(|| panic!("the `gate` job no longer matrixes on `gate`"));
    assert!(
        legs.contains("fromJSON(needs.gates.outputs."),
        "the `gate` matrix legs are `{legs}` — expand the manifest the `gates` job read, or the \
         list is a second declaration of what a gate is."
    );

    let manifest_job = job_lines(&workflow, "gates")
        .unwrap_or_else(|| panic!("ci.yml has no `gates:` job to read the manifest"));
    assert!(
        runs_recipe(&manifest_job, "gates"),
        "ci.yml's `gates` job does not call `just gates`. It has to ask the question the same way \
         `just ci` asks it; a second query is a second answer waiting to happen."
    );
}

#[test]
fn every_gate_runs_from_a_bare_recipe_name() {
    // The manifest prints names, and both readers of it — the matrix leg and
    // `just ci`'s lane loop — run `just <name>` with nothing after it. So the
    // attribute now carries a requirement the recipe alone does not show:
    // tagging `prop-seed SEED` or `fuzz-quick TARGET` writes a CI leg that
    // cannot start.
    let justfile = read("Justfile");
    let manifest = recipes_in_group(&justfile, "gate");
    assert!(
        manifest.len() >= 15,
        "`[group('gate')]` came out as {manifest:?}; the reader is not finding the attribute"
    );
    for gate in &manifest {
        for parameter in recipe_parameters(recipe_header(&justfile, gate)) {
            assert!(
                parameter.contains('=') || parameter.starts_with(['*', '+']),
                "`{gate}` takes the required parameter `{parameter}`, and every gate is invoked as \
                 a bare `just {gate}` — by its matrix leg and by `just ci`. Give it a default."
            );
        }
    }
}

#[test]
fn just_ci_checks_the_manifest_before_it_runs_a_gate() {
    // A mismatch has to cost a second, not a pipeline. `just ci` is what a
    // developer runs instead of waiting for CI; if it reported the drift after
    // the compile lane, the check would arrive later than the answer it is
    // supposed to protect.
    let justfile = read("Justfile");
    let body = ci_recipe_body(&justfile);
    assert!(
        !body.is_empty(),
        "the Justfile has no `ci:` recipe any more"
    );

    let read_at = body
        .iter()
        .position(|line| line_runs_recipe(line, "gates"))
        .unwrap_or_else(|| {
            panic!(
                "`just ci` never runs `just gates`: its lane list is a hand-maintained second \
                 manifest again, and nothing compares the two."
            )
        });
    let dispatch_at = body
        .iter()
        .position(|line| strip_comment(line).contains("just \"$"))
        .unwrap_or_else(|| {
            panic!("`just ci` no longer dispatches its lanes as `just \"$…\"`; teach this test the new shape")
        });
    assert!(
        read_at < dispatch_at,
        "`just ci` reads the manifest {read_at} lines in and starts running gates at {dispatch_at}"
    );
    assert!(
        body[read_at..dispatch_at]
            .iter()
            .any(|line| strip_comment(line).contains("exit 1")),
        "`just ci` compares its lanes against the manifest and carries on regardless — a \
         disagreement has to stop the run, or it is a warning nobody reads."
    );
}

#[test]
fn every_job_the_workflow_defines_is_required_by_ci_success() {
    // Branch protection requires `ci-success` and nothing else, so a job left
    // out of its `needs:` is a job whose failure merges. That was always true;
    // what changed is the blast radius. `gates` computes the matrix, and a
    // failure there does not fail one gate — it produces no legs at all, which
    // is a green PR with nothing behind it.
    let workflow = read(".github/workflows/ci.yml");
    let jobs = job_keys(&workflow);
    assert!(
        jobs.len() >= 5,
        "ci.yml came out with jobs {jobs:?}; the reader is not finding the `jobs:` mapping"
    );

    let required = job_needs(&workflow, "ci-success");
    let unguarded: Vec<&String> = jobs
        .iter()
        .filter(|job| job.as_str() != "ci-success" && !required.contains(*job))
        .collect();
    assert!(
        unguarded.is_empty(),
        "ci.yml defines these jobs and `ci-success` does not wait on them: {unguarded:?}\n\
         Only `ci-success` is a required check, so a job outside its `needs:` can fail into a \
         green PR."
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
fn the_group_reader_counts_attributes_and_not_prose() {
    // Same failure as `a_tool_named_only_in_prose_does_not_count_as_invoked`,
    // one file down: this Justfile explains `[group('gate')]` in a comment
    // above nearly every gate, so a reader that matched text would call the
    // whole file a gate. The `[group('gates')]` recipe is the other trap —
    // that is the aggregate's own group, and a prefix match would enrol
    // `just ci` in the set of things `just ci` has to run.
    let justfile = concat!(
        "# What `[group('gate')]` means and why this recipe is not one.\n",
        "_dev := \"docker compose run --rm dev\"\n",
        "\n",
        "[group('gates')]\n",
        "ci:\n",
        "    echo the aggregate, whose own group is not the manifest\n",
        "\n",
        "[group('gate'), group(\"native\")]\n",
        "commitlint RANGE=\"origin/main..HEAD\":\n",
        "    committed \"{{RANGE}}\"\n",
        "\n",
        "[group('lint')]\n",
        "fmt:\n",
        "    cargo fmt\n",
    );
    let expected = BTreeSet::from(["commitlint".to_owned()]);
    assert_eq!(
        recipes_in_group(justfile, "gate"),
        expected,
        "the gate manifest read something other than the one tagged recipe"
    );
    // Both attributes on one line, and `"native"` in the other quote: `just`
    // accepts each (checked on the pinned 1.51.0 and on latest), so a reader
    // that insisted on one spelling per line would drop a gate the workflow
    // is running.
    assert_eq!(
        recipes_in_group(justfile, "native"),
        expected,
        "a second `group(…)` on the same attribute line, or a double-quoted name, went unread"
    );
    assert!(
        !recipes_in_group(justfile, "gates").contains("commitlint"),
        "`[group('gates')]` and `[group('gate')]` are being read as the same group"
    );
    assert_eq!(
        recipe_parameters(recipe_header(justfile, "commitlint")),
        vec!["RANGE=\"origin/main..HEAD\""],
        "a parameter with a default was read as part of the name, or lost"
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

// ---------------------------------------------------------------------------
// reading what a recipe hands its tool
// ---------------------------------------------------------------------------

/// The tools whose invocation IS the definition of a build step. A workflow
/// that spells one of these out has written a second copy of a build some
/// recipe already owns; a workflow that runs `just <recipe>` has not.
const BUILD_TOOLS: &[&str] = &["cargo", "wasm-pack", "bun"];

/// `NAME := "value"` assignments whose right-hand side is one string literal.
/// `_dev` and its siblings are `if` expressions and stay unresolved on
/// purpose: nothing here asks what they expand to, only what they wrap.
///
/// Either quote, because `just` takes either and the choice is forced by the
/// value: `_NO_COLLISION` holds the `"…"` a `grep` and a `printf` need, so it
/// can only be written in `'…'`. A reader that took double quotes alone would
/// hand `{{_NO_COLLISION}}` on to a shell as text and read a guarded recipe as
/// unguarded.
fn plain_variables(justfile: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in justfile.lines() {
        if line.starts_with([' ', '\t', '#', '[']) {
            continue;
        }
        let Some((name, value)) = line.split_once(":=") else {
            continue;
        };
        let value = value.trim();
        let Some(literal) = quoted_literal(value, '"').or_else(|| quoted_literal(value, '\''))
        else {
            continue;
        };
        out.insert(name.trim().to_owned(), literal.to_owned());
    }
    out
}

/// The inside of `value`, when the whole of it is one `quote`-delimited
/// literal.
fn quoted_literal(value: &str, quote: char) -> Option<&str> {
    value
        .strip_prefix(quote)
        .and_then(|rest| rest.strip_suffix(quote))
}

/// One `{{VAR}}` substitution pass. The deny flag is held in a variable, and a
/// reader that stopped at the interpolation would be reading the name of the
/// policy instead of the policy.
fn expand(line: &str, variables: &BTreeMap<String, String>) -> String {
    let mut out = line.to_owned();
    for (name, value) in variables {
        out = out.replace(&format!("{{{{{name}}}}}"), value);
    }
    out
}

/// The words a shell would see, with quoting and grouping punctuation as
/// separators. It is what lets one reader find `cargo doc` inside
/// `{{_dev}} bash -c 'RUSTDOCFLAGS="…" cargo doc …'`.
fn shell_tokens(line: &str) -> Vec<String> {
    let flattened: String = line
        .chars()
        .map(|ch| {
            if matches!(ch, '"' | '\'' | '`' | '(' | ')' | '{' | '}' | ',') {
                ' '
            } else {
                ch
            }
        })
        .collect();
    flattened.split_whitespace().map(str::to_owned).collect()
}

/// A sub-command reads as one only if it is a bare word: the token after
/// `cargo` in `command -v cargo > /dev/null` is `>`, and that is not a call.
fn is_subcommand_word(token: &str) -> bool {
    token.starts_with(|ch: char| ch.is_ascii_alphabetic())
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
}

/// Every `<tool> <sub-command>` this line runs, for the tools above. A
/// `+nightly` between the two is a toolchain, not the call.
fn tool_commands(line: &str) -> Vec<(String, String)> {
    let tokens = shell_tokens(line);
    let mut out = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if !BUILD_TOOLS.contains(&token.as_str()) {
            continue;
        }
        let mut next = at + 1;
        while tokens.get(next).is_some_and(|token| token.starts_with('+')) {
            next += 1;
        }
        if let Some(sub) = tokens.get(next).filter(|token| is_subcommand_word(token)) {
            out.push((token.clone(), sub.clone()));
        }
    }
    out
}

/// Does this line build documentation? `cargo doc` and `cargo rustdoc` do;
/// `just doc` does not — it runs the recipe that does, which is the whole
/// point of the distinction.
fn builds_rustdoc(line: &str) -> bool {
    tool_commands(line)
        .iter()
        .any(|(tool, sub)| tool == "cargo" && matches!(sub.as_str(), "doc" | "rustdoc"))
}

/// The value a command assigns to `RUSTDOCFLAGS`, quotes resolved. This is the
/// only channel the flags can arrive by: `rustdocflags` would otherwise belong
/// in `.cargo/config.toml`, and `/.cargo/` is git-ignored here because
/// `CARGO_HOME` resolves there inside the dev image.
fn rustdocflags_on(line: &str) -> Option<&str> {
    let after = line.split_once("RUSTDOCFLAGS=")?.1;
    match after.chars().next()? {
        quote @ ('"' | '\'') => after[1..].split(quote).next(),
        _ => after.split_whitespace().next(),
    }
}

/// Do these flags turn a rustdoc warning into a failure?
fn denies_warnings(flags: &str) -> bool {
    let words: Vec<&str> = flags.split_whitespace().collect();
    words
        .iter()
        .any(|word| matches!(*word, "-Dwarnings" | "--deny=warnings"))
        || words
            .windows(2)
            .any(|pair| matches!(pair[0], "-D" | "--deny") && pair[1] == "warnings")
}

/// The indented lines under a recipe header.
fn recipe_body<'a>(justfile: &'a str, recipe: &str) -> Vec<&'a str> {
    let header = recipe_header(justfile, recipe);
    justfile
        .lines()
        .skip_while(|line| *line != header)
        .skip(1)
        .take_while(|line| line.starts_with([' ', '\t']) || line.trim().is_empty())
        .collect()
}

/// A documentation build the `Justfile` defines: the recipe that writes it,
/// the flags it hands rustdoc, and the arguments it hands cargo.
struct DocBuild {
    recipe: String,
    line: String,
    flags: Option<String>,
    arguments: Vec<String>,
}

/// Every command line the `Justfile` runs — comments off, `{{VAR}}`
/// resolved — paired with the recipe it belongs to.
///
/// One walk for both readers below. "What does this recipe hand its tool" is
/// asked here of `cargo doc` and of `cargo fuzz`, and two walks would be two
/// answers to the same question about the same file.
fn expanded_recipe_lines(justfile: &str) -> Vec<(String, String)> {
    let variables = plain_variables(justfile);
    let mut recipe = String::new();
    let mut out = Vec::new();
    for line in justfile.lines() {
        if !line.starts_with([' ', '\t']) {
            // A column-zero line is a header, an assignment, an attribute or a
            // comment. Only the first names the recipe the indented lines
            // under it belong to — and a comment holding a colon would
            // otherwise read as one.
            if let Some(name) = recipe_name(line).filter(|_| !line.starts_with(['#', '['])) {
                recipe = name;
            }
            continue;
        }
        out.push((recipe.clone(), expand(strip_comment(line), &variables)));
    }
    out
}

/// Every documentation build in the `Justfile`, attributed to its recipe.
fn doc_builds(justfile: &str) -> Vec<DocBuild> {
    let mut out = Vec::new();
    for (recipe, expanded) in expanded_recipe_lines(justfile) {
        if !builds_rustdoc(&expanded) {
            continue;
        }
        let flags = rustdocflags_on(&expanded).map(str::to_owned);
        out.push(DocBuild {
            recipe,
            arguments: shell_tokens(&expanded),
            line: expanded,
            flags,
        });
    }
    out
}

/// Every `<tool> <sub-command>` a gate runs, transitively through the recipes
/// it depends on, mapped to the gate that owns it. `playground-build` reaches
/// `wasm-pack build` through two dependencies, and a copy of that command in a
/// workflow is a second definition however far away the original sits.
fn commands_owned_by_gates(justfile: &str) -> BTreeMap<(String, String), String> {
    let variables = plain_variables(justfile);
    let mut out = BTreeMap::new();
    for gate in recipes_in_group(justfile, "gate") {
        let mut pending = vec![gate.clone()];
        let mut seen = BTreeSet::new();
        while let Some(recipe) = pending.pop() {
            if !seen.insert(recipe.clone()) {
                continue;
            }
            pending.extend(header_dependencies(recipe_header(justfile, &recipe)));
            for line in recipe_body(justfile, &recipe) {
                let expanded = expand(strip_comment(line), &variables);
                for command in tool_commands(&expanded) {
                    out.entry(command).or_insert_with(|| gate.clone());
                }
            }
        }
    }
    out
}

/// Does this recipe reach its tool through `docker compose`? Those are the
/// recipes that have to be told where they are when something outside the dev
/// image runs them.
fn recipe_runs_in_a_container(justfile: &str, recipe: &str) -> bool {
    recipe_body(justfile, recipe).iter().any(|line| {
        let body = strip_comment(line);
        [
            "{{_dev}}",
            "{{_ci}}",
            "{{_fuzz}}",
            "{{_pg",
            "docker compose",
        ]
        .iter()
        .any(|marker| body.contains(marker))
    })
}

fn recipe_exists(justfile: &str, recipe: &str) -> bool {
    justfile.lines().any(|line| {
        !line.starts_with([' ', '\t', '#', '[']) && recipe_name(line).as_deref() == Some(recipe)
    })
}

/// Every workflow. Only ci.yml was ever read for a second definition of a
/// gate, and the `cargo doc` this PR deleted was in docs.yml.
fn workflow_files() -> Vec<PathBuf> {
    let dir = repo_root().join(".github/workflows");
    let mut out: Vec<PathBuf> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
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

/// The repo-relative name of a path, for a failure message.
fn label_of(path: &Path) -> String {
    path.strip_prefix(repo_root())
        .unwrap_or(path)
        .display()
        .to_string()
}

// ---------------------------------------------------------------------------
// the doc gates
// ---------------------------------------------------------------------------

#[test]
fn every_rustdoc_build_this_repo_drives_denies_warnings() {
    // rustdoc denies exactly one of its lints by default
    // (`broken_intra_doc_links`); the rest — invalid HTML, a link out of a
    // private item, a redundant explicit link — are warnings, and a
    // `cargo doc` that prints them exits 0. `[workspace.lints.rustdoc]` is
    // what used to make the difference, and it covers two things less than it
    // looks: only the eight lints someone wrote down, and only in a crate
    // whose manifest says `[lints] workspace = true`. Measured, not inferred —
    // drop that opt-in from one crate and an unclosed `<div>` in its docs is a
    // warning `just doc` exits 0 on. `-D warnings` is the half that does not
    // ask which crate or which lint.
    //
    // The assertion is over every documentation build the file defines rather
    // than over the two recipes by name: a third one added without the flag is
    // the same hole reopened, and naming today's recipes would not see it.
    let justfile = read("Justfile");
    let builds = doc_builds(&justfile);
    // A blindness check only — how MANY doc builds there should be is the
    // docs.rs test's question, and a floor that also answered it would report
    // the wrong defect.
    assert!(
        !builds.is_empty(),
        "the Justfile came out with no rustdoc build at all; the reader is not finding \
         `cargo doc` in a recipe body"
    );
    for build in &builds {
        let flags = build.flags.as_deref().unwrap_or("");
        assert!(
            denies_warnings(flags),
            "`just {}` builds documentation with RUSTDOCFLAGS=`{flags}`, which does not deny \
             warnings — so every warn-level rustdoc lint reports and exits 0, and the recipe is \
             a report rather than a gate:\n      {}",
            build.recipe,
            build.line.trim()
        );
    }
}

#[test]
fn the_justfile_is_the_only_place_a_documentation_build_is_written() {
    // The other half of the same defect. `docs.yml` spelled the build out
    // itself, with a `-D warnings` the recipe did not have: two definitions of
    // one command, differing in exactly the direction that lets a regression
    // merge green and fail on the deploy. A workflow that needs the doc build
    // runs `just doc`, so there is one definition to keep strict.
    let mut scanned = 0;
    let root = repo_root();
    let extra = [root.join("Dockerfile"), root.join("bacon.toml")];
    for path in workflow_files().into_iter().chain(extra) {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        for (index, line) in text.lines().enumerate() {
            let body = strip_comment(line);
            let at = index + 1;
            assert!(
                !builds_rustdoc(body),
                "{label}:{at} runs rustdoc itself: `{}`\n\
                 The `doc` / `doc-public` recipes are the definition of that build — call one \
                 with `just`, or this file is a second copy of it that drifts on its own.",
                body.trim()
            );
            assert!(
                !body.contains("RUSTDOCFLAGS"),
                "{label}:{at} sets RUSTDOCFLAGS: `{}`\n\
                 The rustdoc warning policy belongs to the recipe, where every runner of it — CI, \
                 the deploy, a laptop — gets the same one.",
                body.trim()
            );
        }
        scanned += 1;
    }
    assert!(
        scanned >= 5,
        "only {scanned} file(s) scanned; the reader is not finding the workflows"
    );

    // And the positive half: the deploy has to reach the build through the
    // recipe, or deleting the copy just deleted the deploy's docs.
    assert!(
        recipes_invoked(&read(".github/workflows/docs.yml")).contains("doc"),
        "docs.yml no longer runs `just doc`. It publishes rustdoc to Pages, so it either calls \
         the recipe or has grown its own definition of the build again."
    );
}

#[test]
fn a_gate_builds_the_documentation_docs_rs_will_publish() {
    // `--document-private-items` is not a superset. It also SILENCES
    // `private_intra_doc_links`, so a public item linking into a private
    // module passes a gate that documents everything and dangles for every
    // reader of the published docs. While `doc` was the only rustdoc gate,
    // nothing in the repo built what docs.rs builds.
    let justfile = read("Justfile");
    let manifest = recipes_in_group(&justfile, "gate");
    let builds = doc_builds(&justfile);
    let published: Vec<&DocBuild> = builds
        .iter()
        .filter(|build| {
            manifest.contains(&build.recipe)
                && !build
                    .arguments
                    .iter()
                    .any(|argument| argument == "--document-private-items")
        })
        .collect();
    assert!(
        !published.is_empty(),
        "every rustdoc gate passes `--document-private-items`: {:?}. docs.rs does not, so what \
         a PR checks is not what consumers get — and the lints that only fire on the public \
         build fire first for them.",
        builds.iter().map(|build| &build.recipe).collect::<Vec<_>>()
    );

    // The rest of what "the build docs.rs performs" means is not a fact about
    // rustdoc, it is whatever `[package.metadata.docs.rs]` says — so it is read
    // out of the manifests rather than written down here a second time. A list
    // spelled here would be a second copy of the published build, and this
    // test's own history is what that costs: the three arguments above were
    // written by hand, the manifests then declared `all-features = true`, and
    // the assertion went on passing over a gate that documented the default
    // feature set. No crate here has one, so `theme`, `serde`, `miette` and
    // `tsify` — most of the documented surface — were built by no gate at all
    // and rustdoc first ran over them on docs.rs.
    let wanted = docs_rs_requirement();
    assert!(
        wanted.all_features || !wanted.rustdoc_args.is_empty(),
        "no published crate declares a [package.metadata.docs.rs] build, so this test derives \
         nothing and holds the gate to a list written inside itself instead"
    );
    let mut required: Vec<&str> = vec!["--locked", "--workspace", "--no-deps"];
    if wanted.all_features {
        required.push("--all-features");
    }

    for build in published {
        for argument in &required {
            assert!(
                build.arguments.iter().any(|have| have == argument),
                "`just {}` is the docs.rs-equivalent gate and is missing `{argument}`:\n      {}",
                build.recipe,
                build.line.trim()
            );
        }
        let flags = build.flags.as_deref().unwrap_or("");
        assert!(
            contains_run(flags, &wanted.rustdoc_args),
            "`just {}` builds with RUSTDOCFLAGS=`{flags}`, and the published crates ask docs.rs \
             for `rustdoc-args = {:?}`. What consumers read is built with those flags; a gate \
             without them is checking a different configuration.",
            build.recipe,
            wanted.rustdoc_args
        );
    }
}

/// A crate whose documentation trips one warn-by-default rustdoc lint
/// (`invalid_html_tags`) and nothing else.
const PROBE_SOURCE: &str = "//! probe: <div>\n";

/// Numbers the scratch directories so two probes can run side by side.
static PROBE_RUNS: AtomicU32 = AtomicU32::new(0);

/// A fresh directory to build something throwaway in.
fn scratch(label: &str) -> PathBuf {
    let run = PROBE_RUNS.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!("aozora-md-{label}-{}-{run}", process::id()));
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("creating {}: {e}", dir.display()));
    dir
}

/// Does rustdoc accept [`PROBE_SOURCE`] when given these flags? Run rather
/// than read: "`-D warnings` denies" is a claim about a tool, and every other
/// assertion in this file could pass on a Justfile whose flags were a typo.
fn rustdoc_accepts_the_probe(flags: &[&str]) -> bool {
    let dir = scratch("doc-probe");
    let source = dir.join("probe.rs");
    fs::write(&source, PROBE_SOURCE)
        .unwrap_or_else(|e| panic!("writing {}: {e}", source.display()));
    let out = Command::new("rustdoc")
        .arg(&source)
        .arg("--out-dir")
        .arg(dir.join("doc"))
        .args(flags)
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running rustdoc: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where the toolchain is installed."
            )
        });
    let accepted = out.status.success();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    drop(fs::remove_dir_all(&dir));
    assert!(
        accepted || stderr.contains("div"),
        "rustdoc rejected the probe for something other than the lint under test:\n{stderr}"
    );
    accepted
}

#[test]
fn the_flags_the_doc_gates_pass_are_what_makes_a_rustdoc_warning_fail() {
    let justfile = read("Justfile");
    let flag_sets: BTreeSet<String> = doc_builds(&justfile)
        .into_iter()
        .filter_map(|build| build.flags)
        .collect();
    assert!(
        !flag_sets.is_empty(),
        "no documentation build in the Justfile sets RUSTDOCFLAGS at all, so rustdoc runs with \
         its own defaults and its warn-level lints cannot fail a gate"
    );

    // The control, and the reason the flags are load-bearing: the probe is a
    // WARNING to rustdoc itself. Left alone it prints and exits 0 — which is
    // what a workspace crate got whenever the hand-written deny list did not
    // reach it.
    assert!(
        rustdoc_accepts_the_probe(&[]),
        "the probe no longer trips a warn-level rustdoc lint — it now fails on its own, so this \
         test would pass without any flags at all. Pick a lint that is still warn-by-default."
    );
    for flags in &flag_sets {
        let split: Vec<&str> = flags.split_whitespace().collect();
        assert!(
            !rustdoc_accepts_the_probe(&split),
            "RUSTDOCFLAGS=`{flags}` still lets a warn-level rustdoc lint through. The flags are \
             in the recipe but they do not deny, so the gate reports and passes."
        );
    }
}

#[test]
fn the_doc_gates_warning_policy_is_not_parked_in_a_git_ignored_config() {
    // Where the flags would naturally live, and cannot. `/.cargo/` is ignored
    // because `CARGO_HOME` resolves there inside the dev image, so a
    // `rustdocflags` written into `.cargo/config.toml` would be a gate that
    // exists only on the machine that wrote it — green everywhere it was
    // never installed.
    let ignored = read(".gitignore")
        .lines()
        .any(|line| line.trim() == "/.cargo/");
    assert!(
        ignored,
        "`/.cargo/` is no longer git-ignored. If a tracked `.cargo/config.toml` is possible now, \
         `rustdocflags` belongs there and the env-var route the recipes take can be retired — \
         but decide it, do not leave both."
    );
    let config = repo_root().join(".cargo/config.toml");
    if let Ok(text) = fs::read_to_string(&config) {
        assert!(
            !text.to_lowercase().contains("rustdocflags"),
            "{} carries rustdocflags, and git ignores that path: the doc gates would deny \
             warnings only for whoever has this file.",
            label_of(&config)
        );
    }
}

// ---------------------------------------------------------------------------
// one definition of a build, wherever it is written
// ---------------------------------------------------------------------------

/// Workflow steps that spell out a build a gate already defines, each with the
/// reason it cannot call the recipe instead. Every row is a second definition
/// of one command — the arrangement removed here for rustdoc and still in
/// place for these.
const RE_SPELLED_BUILD: &[(&str, &str, &str, &str)] = &[
    (
        ".github/workflows/docs.yml",
        "wasm-pack",
        "build",
        "`wasm-build` wraps wasm-pack in `{{_dev}}`, and the Pages job has no dev image — \
         it builds on the native runner so the artefacts land where the upload can see them.",
    ),
    (
        ".github/workflows/docs.yml",
        "bun",
        "install",
        "`playground-install` hard-codes `docker compose run --rm playground` rather than \
         going through the `_in` switch, so unlike `just doc` it cannot be run natively at all.",
    ),
    (
        ".github/workflows/docs.yml",
        "bun",
        "run",
        "`playground-build`, same reason as the `bun install` above.",
    ),
];

#[test]
fn no_workflow_spells_out_a_build_a_gate_recipe_already_defines() {
    // `the_workflow_hand_writes_no_gate_of_its_own` asks this of ci.yml, by
    // NAME. Both narrowings mattered: the duplicate lived in docs.yml, and it
    // never named the `doc` gate — it wrote the gate's command out instead.
    // What a check is, is the command it runs, so that is what has to be
    // single-sourced.
    let justfile = read("Justfile");
    let owned = commands_owned_by_gates(&justfile);
    assert!(
        owned.len() >= 8,
        "the gates came out running {owned:?}; the reader is not finding their bodies"
    );

    let mut re_spelled = Vec::new();
    for path in workflow_files() {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        for line in jobs_block(&text) {
            let body = strip_comment(line);
            for (tool, sub) in tool_commands(body) {
                let Some(gate) = owned.get(&(tool.clone(), sub.clone())) else {
                    continue;
                };
                if RE_SPELLED_BUILD
                    .iter()
                    .any(|&(file, owner, name, _)| file == label && owner == tool && name == sub)
                {
                    continue;
                }
                re_spelled.push(format!(
                    "{label}: `{tool} {sub}` is the build `just {gate}` defines\n      {}",
                    body.trim()
                ));
            }
        }
    }
    assert!(
        re_spelled.is_empty(),
        "workflow steps that re-spell a gate's build:\n{}\n\
         Run the recipe instead, or add the step to RE_SPELLED_BUILD with the reason it cannot — \
         a second copy drifts from the first in whichever direction nobody is looking.",
        re_spelled.join("\n")
    );
}

#[test]
fn a_workflow_that_runs_a_containerized_recipe_says_where_it_is() {
    // The seam calling the recipe opened. Every recipe here reaches its tool
    // through `docker compose run`, and `AOZORA_MD_IN_CONTAINER=1` is what
    // makes the `_in` switch resolve it directly instead. A job on a bare
    // runner that forgets it does not fail loudly — it builds inside a
    // container whose filesystem the next step cannot read.
    //
    // ci.yml's two native jobs were checked for this already; the rule is not
    // about being native, it is about being outside the dev image, which is
    // every job that does not pull it.
    let justfile = read("Justfile");
    let mut asked = 0;
    for path in workflow_files() {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        for job in job_keys(&text) {
            let Some(lines) = job_lines(&text, &job) else {
                continue;
            };
            let pulls_the_image = lines
                .iter()
                .any(|line| strip_comment(line).contains("setup-dev-image"));
            for recipe in recipes_invoked_in(&lines) {
                assert!(
                    recipe_exists(&justfile, &recipe),
                    "{label}'s `{job}` job runs `just {recipe}` and the Justfile has no such \
                     recipe any more"
                );
                if pulls_the_image || !recipe_runs_in_a_container(&justfile, &recipe) {
                    continue;
                }
                asked += 1;
                assert!(
                    lines.iter().any(|line| strip_comment(line)
                        .trim()
                        .starts_with("AOZORA_MD_IN_CONTAINER:")),
                    "{label}'s `{job}` job runs `just {recipe}` without pulling the dev image and \
                     without setting AOZORA_MD_IN_CONTAINER. The recipe will wrap its tool in \
                     `docker compose run`, so the job tests an image it did not build and writes \
                     its output where the next step cannot find it."
                );
            }
        }
    }
    // Two before this PR (`msrv`, `commitlint`), three with docs.yml's
    // `just doc`. The floor says the reader still sees such a job at all, not
    // how many there are: a workflow deleting its recipe call is that
    // workflow's business, a reader that stopped finding them is this test's.
    assert!(
        asked >= 2,
        "only {asked} job(s) run a containerized recipe outside the dev image; the reader is not \
         finding them"
    );
}

// ---------------------------------------------------------------------------
// what the doc readers claim, pinned both ways
// ---------------------------------------------------------------------------

#[test]
fn a_doc_build_without_the_deny_is_read_as_one_and_with_it_is_not() {
    let justfile = concat!(
        "_DENY := \"-D warnings\"\n",
        "\n",
        "# Build the docs, and mind the `cargo doc` in this comment.\n",
        "[group('gate')]\n",
        "doc:\n",
        "    {{_dev}} bash -c 'cargo doc --locked --workspace --document-private-items'\n",
        "\n",
        "doc-public:\n",
        "    {{_dev}} bash -c 'RUSTDOCFLAGS=\"{{_DENY}}\" cargo doc --locked --workspace'\n",
        "\n",
        "publish:\n",
        "    {{_dev}} just doc\n",
    );
    let builds = doc_builds(justfile);
    assert_eq!(
        builds.len(),
        2,
        "a `cargo doc` in a comment, or a `just doc`, was counted as a build: {:?}",
        builds.iter().map(|build| &build.line).collect::<Vec<_>>()
    );
    assert_eq!(
        builds[0].recipe, "doc",
        "a build was attributed to the wrong recipe"
    );
    assert!(
        builds[0].flags.is_none(),
        "a recipe with no RUSTDOCFLAGS came out carrying some"
    );
    assert_eq!(
        builds[1].flags.as_deref(),
        Some("-D warnings"),
        "the interpolated deny flag was not resolved through the variable"
    );
    assert!(
        denies_warnings(builds[1].flags.as_deref().unwrap_or("")),
        "the resolved flags did not read as denying warnings"
    );
    assert!(
        builds[1]
            .arguments
            .iter()
            .all(|argument| argument != "--document-private-items"),
        "the docs.rs-shaped build was read as documenting private items"
    );
}

#[test]
fn a_deny_that_covers_something_other_than_warnings_is_not_the_policy() {
    // The flags are a free-form string, so "it is set" is not the question.
    for spelling in ["-D warnings", "--cfg docsrs -Dwarnings", "--deny warnings"] {
        assert!(
            denies_warnings(spelling),
            "`{spelling}` denies warnings and was not read as doing so"
        );
    }
    assert!(
        !denies_warnings(""),
        "empty flags counted as a warning policy"
    );
    assert!(
        !denies_warnings("-W warnings"),
        "a warn-level flag counted as a deny"
    );
    assert!(
        !denies_warnings("-D broken_intra_doc_links"),
        "denying one lint counted as denying the class — the point of the flag is the lints \
         nobody has listed, including the ones that do not exist yet"
    );
    assert!(
        !denies_warnings("-A warnings"),
        "an allow counted as a deny"
    );
}

#[test]
fn a_recipes_own_command_is_owned_and_a_recipe_it_depends_on_is_too() {
    let justfile = concat!(
        "[group('gate')]\n",
        "playground-build: playground-install\n",
        "    {{_pg}} bash -c 'bun run build'\n",
        "\n",
        "playground-install: wasm-build\n",
        "    {{_pg}} bash -c 'bun install --frozen-lockfile'\n",
        "\n",
        "wasm-build:\n",
        "    {{_dev}} bash -c 'wasm-pack build crates/x -- --locked'\n",
        "\n",
        "[group('lint')]\n",
        "fmt:\n",
        "    {{_dev}} cargo fmt --all\n",
    );
    let owned = commands_owned_by_gates(justfile);
    for command in [("bun", "run"), ("bun", "install"), ("wasm-pack", "build")] {
        let key = (command.0.to_owned(), command.1.to_owned());
        assert_eq!(
            owned.get(&key).map(String::as_str),
            Some("playground-build"),
            "`{} {}` was not traced back to the gate that depends on it",
            command.0,
            command.1
        );
    }
    assert!(
        !owned.contains_key(&("cargo".to_owned(), "fmt".to_owned())),
        "a recipe no gate depends on had its command counted as gate-owned"
    );
}

// ---------------------------------------------------------------------------
// what a cargo manifest declares
// ---------------------------------------------------------------------------
//
// Everything above reads a Justfile or a workflow. Two of the properties this
// repo leans on are declared somewhere no recipe can reach: a lint table that
// covers a crate only if that crate's own manifest opts in, and a docs.rs
// build configured per published crate and performed on a machine none of
// these gates run on. Both are the shape this file exists for — a declaration
// whose execution is somewhere else — so they are read the same way.

/// The table a `[header]` line opens, or `None` for any other line. `[[bin]]`
/// reads as `bin`: an array-of-tables is still that table.
fn table_header(line: &str) -> Option<&str> {
    let trimmed = strip_comment(line).trim();
    let inner = trimmed.strip_prefix('[')?.strip_suffix(']')?;
    Some(inner.trim_start_matches('[').trim_end_matches(']'))
}

/// A `key = value` line, comment stripped. A dotted key (`version.workspace`)
/// keeps its dots, so it never answers for the bare key beside it.
fn manifest_pair(line: &str) -> Option<(&str, &str)> {
    let (key, value) = strip_comment(line).split_once('=')?;
    Some((key.trim(), value.trim()))
}

/// Every `key = value` inside one table of a manifest.
fn table_pairs<'a>(manifest: &'a str, table: &str) -> Vec<(&'a str, &'a str)> {
    let mut current = "";
    let mut out = Vec::new();
    for line in manifest.lines() {
        if let Some(header) = table_header(line) {
            current = header;
            continue;
        }
        if current != table {
            continue;
        }
        if let Some(pair) = manifest_pair(line) {
            out.push(pair);
        }
    }
    out
}

/// The value `key` takes inside `[table]`.
fn manifest_value<'a>(manifest: &'a str, table: &str, key: &str) -> Option<&'a str> {
    table_pairs(manifest, table)
        .into_iter()
        .find(|&(name, _)| name == key)
        .map(|(_, value)| value)
}

/// The quoted strings of a single-line TOML value: an array's items, or the
/// one string a scalar holds.
fn quoted_items(value: &str) -> Vec<String> {
    value
        .split('"')
        .skip(1)
        .step_by(2)
        .map(str::to_owned)
        .collect()
}

/// A workspace member: the directory the manifest sits in, and that manifest.
struct Member {
    path: String,
    manifest: String,
}

/// Every member the workspace manifest lists, with its manifest read. Derived
/// rather than listed here for the same reason the gate manifest is: a crate
/// added and forgotten is the one this file has to see.
fn workspace_members() -> Vec<Member> {
    let root = read("Cargo.toml");
    let mut paths: Vec<String> = Vec::new();
    let mut inside = false;
    for line in root.lines() {
        let body = strip_comment(line);
        if !inside {
            inside = body.trim_start().starts_with("members") && body.contains('[');
            continue;
        }
        if body.trim_start().starts_with(']') {
            break;
        }
        paths.extend(quoted_items(body));
    }
    assert!(
        paths.len() >= 5,
        "only {paths:?} read out of the workspace manifest's `members`; the reader is not \
         finding the list"
    );
    paths
        .into_iter()
        .map(|path| {
            let manifest = read(&format!("{path}/Cargo.toml"));
            Member { path, manifest }
        })
        .collect()
}

/// Does this member reach crates.io? `publish = false` is the opt-out cargo
/// honours and the only one any member here uses.
fn is_published(manifest: &str) -> bool {
    manifest_value(manifest, "package", "publish") != Some("false")
}

/// What a crate's manifest asks docs.rs to build for it.
#[derive(Debug, Default, PartialEq, Eq)]
struct DocsRsBuild {
    all_features: bool,
    rustdoc_args: Vec<String>,
}

/// The `[package.metadata.docs.rs]` table, or `None` when the crate declares
/// none — in which case docs.rs builds default features and stops there.
fn docs_rs_build(manifest: &str) -> Option<DocsRsBuild> {
    const TABLE: &str = "package.metadata.docs.rs";
    let declared = manifest
        .lines()
        .any(|line| table_header(line) == Some(TABLE));
    declared.then(|| DocsRsBuild {
        all_features: manifest_value(manifest, TABLE, "all-features") == Some("true"),
        rustdoc_args: manifest_value(manifest, TABLE, "rustdoc-args")
            .map(quoted_items)
            .unwrap_or_default(),
    })
}

/// The build the published crates ask docs.rs to perform, as one answer.
/// `all-features` counts only when every one of them says it: a gate passing
/// `--all-features` on behalf of a crate whose manifest does not would be
/// checking a configuration nobody publishes.
fn docs_rs_requirement() -> DocsRsBuild {
    let declared: Vec<DocsRsBuild> = workspace_members()
        .iter()
        .filter(|member| is_published(&member.manifest))
        .filter_map(|member| docs_rs_build(&member.manifest))
        .collect();
    DocsRsBuild {
        all_features: !declared.is_empty() && declared.iter().all(|build| build.all_features),
        rustdoc_args: declared
            .first()
            .map(|build| build.rustdoc_args.clone())
            .unwrap_or_default(),
    }
}

/// Does `flags` carry `wanted` as a run of consecutive words? `--cfg docsrs`
/// is two words that mean nothing apart, so they have to arrive together and
/// in that order.
fn contains_run(flags: &str, wanted: &[String]) -> bool {
    let words: Vec<&str> = flags.split_whitespace().collect();
    wanted.is_empty()
        || words.windows(wanted.len()).any(|window| {
            window
                .iter()
                .zip(wanted)
                .all(|(word, want)| *word == want.as_str())
        })
}

#[test]
fn every_crate_this_repo_publishes_declares_the_docs_rs_build_it_wants() {
    // docs.rs builds default features unless a crate says otherwise, and every
    // feature in this workspace is off by default: `theme`'s stylesheets, the
    // `serde` derives, the `miette` impl, the `tsify` bindings. A published
    // crate with no `[package.metadata.docs.rs]` therefore ships a page
    // missing most of what a reader opened it for — and nothing here could
    // have reported it, because the build that would show it runs on docs.rs,
    // after publication, with no gate in front of it.
    //
    // Over every member rather than the four crates that publish today. The
    // fifth is precisely the one that gets forgotten, and it would be
    // forgotten in the direction nothing notices.
    let members = workspace_members();
    let mut published = 0_usize;
    let mut opted_out = 0_usize;
    let mut args: Vec<(String, Vec<String>)> = Vec::new();
    for member in &members {
        if !is_published(&member.manifest) {
            opted_out += 1;
            continue;
        }
        published += 1;
        let build = docs_rs_build(&member.manifest).unwrap_or_else(|| {
            panic!(
                "{}/Cargo.toml publishes to crates.io and declares no \
                 [package.metadata.docs.rs]. Its page would document the default feature set, \
                 which in this workspace means: none of them.",
                member.path
            )
        });
        assert!(
            build.all_features,
            "{}/Cargo.toml declares [package.metadata.docs.rs] without `all-features = true`, \
             so docs.rs builds its default features — and no crate here has any.",
            member.path
        );
        args.push((member.path.clone(), build.rustdoc_args));
    }

    // Blindness check on the reader, both ways: it has to find crates that
    // publish AND crates that do not, or `publish = false` is not being read
    // and every assertion above was skipped or vacuous.
    assert!(
        published >= 1 && opted_out >= 1,
        "{published} published and {opted_out} opted-out member(s) out of {}; the reader is not \
         telling `publish = false` apart from its absence",
        members.len()
    );

    // One build for all of them. Four pages built four ways is four
    // configurations to reason about, and only one of them can be the one a
    // gate here reproduces.
    for pair in args.windows(2) {
        assert_eq!(
            pair[0].1, pair[1].1,
            "{} and {} ask docs.rs for different `rustdoc-args`. A gate can reproduce one \
             published build, so they have to be one build.",
            pair[0].0, pair[1].0
        );
    }
}

#[test]
fn every_lint_this_workspace_declares_reaches_every_crate_it_builds() {
    // `[workspace.lints]` is a declaration with a per-crate opt-in, and the
    // opt-in fails open. A member whose manifest omits `[lints] workspace =
    // true` inherits none of it — not `unsafe_code = "forbid"`, not the eight
    // rustdoc denials, not `missing_docs` — and nothing reports that. The
    // crate simply stops being covered while `just clippy` and `just
    // doc-public` stay green over it, which is the state this file's own
    // header describes and no assertion in it had yet checked.
    let members = workspace_members();
    for member in &members {
        assert_eq!(
            manifest_value(&member.manifest, "lints", "workspace"),
            Some("true"),
            "{}/Cargo.toml carries no `[lints] workspace = true`, so [workspace.lints] reaches \
             it with nothing at all and every lint this repo relies on is off there.",
            member.path
        );
    }

    // And the declaration the opt-in distributes. `missing_docs` is what makes
    // the published surface explain itself; it is warn-level because a bare
    // `cargo build` should stay usable mid-edit, and `just clippy`'s
    // `-D warnings` is what turns it into a gate — so `allow`, or absence,
    // silently reopens every undocumented public item.
    let root = read("Cargo.toml");
    let rust_lints = table_pairs(&root, "workspace.lints.rust");
    assert!(
        rust_lints.len() >= 5,
        "[workspace.lints.rust] came out as {rust_lints:?}; the reader is not finding the table"
    );
    let level = rust_lints
        .iter()
        .find(|&&(key, _)| key == "missing_docs")
        .map(|&(_, value)| value);
    assert!(
        matches!(level, Some(set) if set.contains("warn") || set.contains("deny")),
        "[workspace.lints.rust] sets missing_docs to {level:?}. Undocumented public items are \
         then nobody's failure: rustdoc does not mind them, clippy does not either, and the \
         first reader to notice is looking at the published page."
    );
}

/// A rustc lint level, ordered by how much it stops.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum LintLevel {
    Allow,
    Warn,
    Deny,
    Forbid,
}

/// The level a `[lints]` entry sets, in either spelling cargo takes: the bare
/// `"warn"` and the table `{ level = "warn", priority = -1 }`. The priority is
/// not read — it orders a group against its own members and says nothing about
/// how much the entry stops.
fn lint_level(value: &str) -> Option<LintLevel> {
    quoted_items(value)
        .iter()
        .find_map(|item| match item.as_str() {
            "allow" => Some(LintLevel::Allow),
            "warn" => Some(LintLevel::Warn),
            "deny" => Some(LintLevel::Deny),
            "forbid" => Some(LintLevel::Forbid),
            _ => None,
        })
}

/// Every directory in the repo holding a `Cargo.toml` with a `[package]`
/// table, repo-relative. Walked rather than listed: a crate this file has to
/// see is by definition one the workspace manifest does not mention.
fn crate_directories() -> BTreeSet<String> {
    /// Directories nobody here authors a crate in.
    const SKIPPED: &[&str] = &["target", ".git", "node_modules", "coverage", "dist"];

    fn walk(dir: &Path, out: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).into_iter().flatten().flatten() {
            let path = entry.path();
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            if path.is_dir() {
                if !SKIPPED.contains(&name) {
                    walk(&path, out);
                }
                continue;
            }
            if name != "Cargo.toml" {
                continue;
            }
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            if text
                .lines()
                .any(|line| table_header(line) == Some("package"))
            {
                out.insert(label_of(dir));
            }
        }
    }

    let mut out = BTreeSet::new();
    walk(&repo_root(), &mut out);
    out
}

/// Lints a crate outside the workspace may leave out of its own `[lints.rust]`
/// table, each with the reason it is not worth re-typing there.
///
/// A table rather than a rule, because "which lints matter to a crate that
/// publishes nothing and exposes nothing" is a judgement and judgements go
/// where the next reader finds them. What it buys is the thing the manifest
/// comment says cannot be bought: a lint ADDED to `[workspace.lints.rust]`
/// from here on lands on this list as an unanswered question rather than
/// silently skipping the one crate that cannot inherit it.
const LINTS_A_NON_MEMBER_CRATE_MAY_SKIP: &[(&str, &str)] = &[
    // Surface policy for a published library. This crate is four `#![no_main]`
    // binaries whose whole body is one `fuzz_target!` invocation: it has no
    // API, no types a consumer names, and no lifetimes written by hand.
    ("missing_debug_implementations", SKIP_NO_SURFACE),
    ("missing_copy_implementations", SKIP_NO_SURFACE),
    ("unreachable_pub", SKIP_NO_SURFACE),
    ("single_use_lifetimes", SKIP_NO_SURFACE),
    ("elided_lifetimes_in_paths", SKIP_NO_SURFACE),
    ("redundant_lifetimes", SKIP_NO_SURFACE),
    ("explicit_outlives_requirements", SKIP_NO_SURFACE),
    ("unused_lifetimes", SKIP_NO_SURFACE),
    ("variant_size_differences", SKIP_NO_SURFACE),
    // Style rules that are gates in the workspace only because `just clippy`
    // passes `-D warnings`. `cargo fuzz build` has no such flag and no way to
    // be given one that would not also apply to every dependency it compiles,
    // so at warn level here they would print into a build log and fail nothing.
    ("trivial_casts", SKIP_NO_DENY_CHANNEL),
    ("trivial_numeric_casts", SKIP_NO_DENY_CHANNEL),
    ("unused_import_braces", SKIP_NO_DENY_CHANNEL),
    ("unused_qualifications", SKIP_NO_DENY_CHANNEL),
    ("let_underscore_drop", SKIP_NO_DENY_CHANNEL),
    ("ambiguous_negative_literals", SKIP_NO_DENY_CHANNEL),
    // Edition drift, which is a warning about source that has to be carried
    // forward. These harnesses are fifteen lines each and are rewritten
    // whenever the API they call moves — which is what `just fuzz-build`
    // exists to make somebody notice.
    ("keyword_idents_2024", SKIP_EDITION_DRIFT),
    ("rust_2024_compatibility", SKIP_EDITION_DRIFT),
];

const SKIP_NO_SURFACE: &str = "API-surface policy for a published crate; this one publishes nothing and \
                       exposes nothing";
const SKIP_NO_DENY_CHANNEL: &str = "style policy that is a gate only through `just clippy`'s `-D warnings`, a \
                     channel `cargo fuzz build` does not have";
const SKIP_EDITION_DRIFT: &str = "edition-drift warning for source that is carried forward; these harnesses \
                       are rewritten whenever the API they call moves";

#[test]
fn every_lint_this_workspace_declares_reaches_the_crate_that_cannot_inherit_it() {
    // The test above says every lint reaches "every crate it builds" and asks
    // it of `workspace_members()`. The fuzz crate is not a member — it declares
    // its own `[workspace]`, and it has to, because libfuzzer-sys is
    // nightly-only — so it was the one crate in the repo that could not opt in
    // and the one crate the net could not see. Nothing carried over: not the
    // rustdoc denials, not `missing_docs`, not `unsafe_code = "forbid"`, which
    // every other crate here forbids and which that one compiled without for as
    // long as it existed (DEV-312).
    //
    // Discovered by walking rather than by naming the fuzz crate, because the
    // property is about crates the members list does not mention, and a second
    // one added tomorrow would be invisible to a check that spelled the first.
    let members: BTreeSet<String> = workspace_members()
        .into_iter()
        .map(|member| member.path)
        .collect();
    let outsiders: Vec<String> = crate_directories()
        .into_iter()
        .filter(|dir| !members.contains(dir))
        .collect();
    assert!(
        !outsiders.is_empty(),
        "no crate outside the workspace members was found; the walk is not reading manifests, \
         and this whole test is then vacuous"
    );

    let root = read("Cargo.toml");
    let declared: BTreeMap<&str, LintLevel> = table_pairs(&root, "workspace.lints.rust")
        .into_iter()
        .filter_map(|(lint, value)| Some((lint, lint_level(value)?)))
        .collect();
    assert!(
        declared.len() >= 15,
        "[workspace.lints.rust] came out as {declared:?}; the reader is not finding the table or \
         is not reading its levels"
    );

    for dir in &outsiders {
        let manifest = read(&format!("{dir}/Cargo.toml"));
        let own: BTreeMap<&str, LintLevel> = table_pairs(&manifest, "lints.rust")
            .into_iter()
            .filter_map(|(lint, value)| Some((lint, lint_level(value)?)))
            .collect();
        assert!(
            !own.is_empty(),
            "{dir}/Cargo.toml declares no `[lints.rust]` at all. It is outside the workspace, so \
             `[lints] workspace = true` cannot reach it and there is nothing else that can: the \
             crate compiles under no lint policy whatever, `unsafe_code` included."
        );

        for (&lint, &wanted) in &declared {
            if let Some(&have) = own.get(lint) {
                assert!(
                    have >= wanted,
                    "{dir}/Cargo.toml sets `{lint}` to {have:?} and the workspace sets it to \
                     {wanted:?}. A crate outside the members list may be stricter than the \
                     workspace — it has no `-D warnings` to lean on — but not looser."
                );
                continue;
            }
            let excused = LINTS_A_NON_MEMBER_CRATE_MAY_SKIP
                .iter()
                .any(|&(name, _)| name == lint);
            assert!(
                excused,
                "[workspace.lints.rust] declares `{lint}` at {wanted:?} and {dir}/Cargo.toml does \
                 not. That crate cannot write `[lints] workspace = true`, so the only way a lint \
                 reaches it is by being re-typed there. Add it, or add it to \
                 LINTS_A_NON_MEMBER_CRATE_MAY_SKIP with the reason it does not apply."
            );
        }
    }

    // A skip for a lint the workspace no longer declares is a sentence
    // excusing nothing, and the next reader takes it for coverage.
    for &(lint, why) in LINTS_A_NON_MEMBER_CRATE_MAY_SKIP {
        assert!(
            declared.contains_key(lint),
            "`{lint}` is excused (\"{why}\") and [workspace.lints.rust] does not declare it"
        );
    }
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn a_lint_table_entry_reads_as_its_level_in_either_spelling() {
    // The workspace writes one entry as a table so it can carry a priority,
    // and a reader that only understood the bare string would call
    // `rust_2024_compatibility` undeclared — i.e. would excuse the fuzz crate
    // from a lint nobody had decided to excuse it from.
    let manifest = concat!(
        "[workspace.lints.rust]\n",
        "unsafe_code = \"forbid\"\n",
        "rust_2024_compatibility = { level = \"warn\", priority = -1 }\n",
        "dead_code = \"deny\"\n",
    );
    let read_here: BTreeMap<&str, Option<LintLevel>> =
        table_pairs(manifest, "workspace.lints.rust")
            .into_iter()
            .map(|(lint, value)| (lint, lint_level(value)))
            .collect();
    assert_eq!(
        read_here.get("rust_2024_compatibility"),
        Some(&Some(LintLevel::Warn)),
        "a table-valued lint entry went unread: {read_here:?}"
    );
    assert_eq!(
        read_here.get("unsafe_code"),
        Some(&Some(LintLevel::Forbid)),
        "a bare lint entry went unread: {read_here:?}"
    );
    // And the order the comparison depends on: "stricter is fine, looser is
    // not" is only a rule if these sort.
    assert!(
        LintLevel::Forbid > LintLevel::Deny
            && LintLevel::Deny > LintLevel::Warn
            && LintLevel::Warn > LintLevel::Allow,
        "the levels do not order, so `deny` where the workspace says `warn` would read as a \
         downgrade"
    );
}

#[test]
fn a_dotted_key_does_not_answer_for_the_bare_key() {
    // Every member spells its inherited fields `version.workspace = true`. A
    // reader that matched on a prefix would read `publish.workspace` — or
    // `version` — as `publish` and call a published crate opted out.
    let manifest = "[package]\nversion.workspace = true\npublish.workspace = true\n";
    assert_eq!(manifest_value(manifest, "package", "publish"), None);
    assert_eq!(manifest_value(manifest, "package", "version"), None);
    assert!(is_published(manifest));
}

#[test]
fn a_key_in_a_neighbouring_table_is_not_in_this_one() {
    // `[package.metadata.dist]` sits beside `[package.metadata.docs.rs]` in
    // the epub CLI's manifest, and it holds a `dist = false` that a reader
    // ignoring table boundaries would hand to whoever asked next.
    let manifest = concat!(
        "[package.metadata.dist]\n",
        "dist = false\n",
        "[package.metadata.docs.rs]\n",
        "all-features = true\n",
    );
    assert_eq!(
        manifest_value(manifest, "package.metadata.dist", "all-features"),
        None
    );
    assert_eq!(
        manifest_value(manifest, "package.metadata.docs.rs", "all-features"),
        Some("true")
    );
}

#[test]
fn a_crate_declaring_no_docs_rs_table_asks_for_no_build() {
    assert_eq!(docs_rs_build("[package]\nname = \"x\"\n"), None);
    assert_eq!(
        docs_rs_build(
            "[package.metadata.docs.rs]\nall-features = true\nrustdoc-args = [\"--cfg\", \"docsrs\"]\n"
        ),
        Some(DocsRsBuild {
            all_features: true,
            rustdoc_args: vec!["--cfg".to_owned(), "docsrs".to_owned()],
        })
    );
}

#[test]
fn a_flag_pair_counts_only_where_it_arrives_whole_and_in_order() {
    let wanted = vec!["--cfg".to_owned(), "docsrs".to_owned()];
    assert!(contains_run("-D warnings --cfg docsrs", &wanted));
    assert!(!contains_run("-D warnings --cfg", &wanted));
    assert!(!contains_run("--cfg feature docsrs", &wanted));
    assert!(!contains_run("docsrs --cfg", &wanted));
    // A crate asking for no rustdoc-args constrains nothing.
    assert!(contains_run("-D warnings", &[]));
}

// ---------------------------------------------------------------------------
// what a gate is excused from
// ---------------------------------------------------------------------------
//
// A fourth way a check can check nothing, and the one this file did not
// cover: the check runs, and something is exempted from it on the strength of
// a sentence. `_COV_IGNORE` drops files out of the coverage denominator, and
// each entry is written down with a reason. Prose counting as enforcement is
// what the header of this file rejects, and an exemption is where it costs the
// most — the gate goes green faster for it.
//
// Two questions settle an exemption, and both are asked below of the regex the
// gate hands llvm-cov rather than of the paragraph above it:
//
//   * Does it hide source this repo publishes? Nothing else measures a
//     published `src/` file, so an exclusion that reaches one takes that file
//     out of every gate at once. Four were in exactly that position — both CLI
//     `main.rs` entry points and the epub serialisation pair — and this
//     section did not see them, because it asked what SHAPE an exclusion had
//     (a bare directory, or "a pattern, therefore policy, therefore not mine
//     to resolve") instead of asking which files it matches. Shape is the
//     wrong question twice over: it let a pattern hide published source, and
//     it would have let the wasm exemption below escape the reader written for
//     it just by being spelled as a pattern over that crate's files.
//   * Does it defer to a step that exists? A crate compiled for a target
//     `cargo llvm-cov` cannot instrument is genuinely invisible to the
//     coverage gate, and the exclusion is honest exactly when something else
//     runs that crate's tests on that target.
//
// So the reader matches every member's `src/` files against the exclusions and
// answers with files. `target/` is the one entry that matches none, which is
// the whole of its claim to be there: it is not source.

/// Split a regex alternation at the top level. A `|` inside a group — the
/// shape `_COV_IGNORE` carried for `epub/src/(compose|package)\.rs` —
/// separates two file names within ONE exclusion, not two exclusions.
fn split_alternatives(regex: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut depth = 0usize;
    let mut start = 0usize;
    for (at, ch) in regex.char_indices() {
        match ch {
            '(' => depth += 1,
            ')' => depth = depth.saturating_sub(1),
            '|' if depth == 0 => {
                out.push(&regex[start..at]);
                start = at + 1;
            }
            _ => {}
        }
    }
    out.push(&regex[start..]);
    out
}

/// What the coverage gate tells llvm-cov to leave out of the denominator.
///
/// Read off the flag the gate hands the tool rather than off the variable
/// holding it: a `_COV_IGNORE` that nothing interpolates excuses nothing, and
/// the question here is what is actually excused.
fn coverage_exclusions(justfile: &str) -> Vec<String> {
    let variables = plain_variables(justfile);
    for line in recipe_body(justfile, "coverage") {
        let expanded = expand(strip_comment(line), &variables);
        let Some((_, after)) = expanded.split_once("--ignore-filename-regex") else {
            continue;
        };
        let argument = after.trim();
        let regex = match argument.chars().next() {
            Some(quote @ ('\'' | '"')) => argument[1..].split(quote).next().unwrap_or_default(),
            _ => argument.split_whitespace().next().unwrap_or_default(),
        };
        let body = regex
            .strip_prefix('(')
            .and_then(|rest| rest.strip_suffix(')'))
            .unwrap_or(regex);
        return split_alternatives(body)
            .into_iter()
            .map(unescaped)
            .collect();
    }
    Vec::new()
}

/// One `"…"` value as `just` hands it on, from the literal the `Justfile`
/// spells. `\\` there is one backslash and `\"` is one quote, so the
/// `(compose|package)\\.rs` written on disk is the `\.` llvm-cov receives.
///
/// A reader that skipped this step would hold a pattern for a literal
/// backslash, match no file with it, and report the exemption it cannot read
/// as harmless — the failure mode this whole section exists to refuse.
fn unescaped(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut chars = literal.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\\' && matches!(chars.peek(), Some('\\' | '"')) {
            continue;
        }
        out.push(ch);
    }
    out
}

/// Every `.rs` file under one directory, at any depth.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let entries = fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    let mut out = Vec::new();
    for entry in entries {
        let path = entry
            .unwrap_or_else(|e| panic!("reading an entry of {}: {e}", dir.display()))
            .path();
        if path.is_dir() {
            out.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// One spelling for a crate directory, so `crates/x`, `./crates/x` and
/// `crates/x/` are one answer whichever file wrote them.
fn crate_dir(path: &str) -> &str {
    path.trim_start_matches("./").trim_end_matches('/')
}

/// Every `src/` file of every workspace member, as `(member, path)` with both
/// relative to the repo root.
///
/// Off the filesystem rather than a list written here: a file added to a crate
/// is measured, or excused, the moment it exists, and a reader working from
/// names would answer for the repo as it was when the names were typed.
fn member_sources() -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    for member in workspace_members() {
        let directory = root.join(&member.path).join("src");
        let files = rust_files(&directory);
        assert!(
            !files.is_empty(),
            "no `.rs` file under {} — a member whose source this reader cannot find is a member \
             it would report as fully measured",
            directory.display()
        );
        for file in files {
            let relative = file
                .strip_prefix(&root)
                .unwrap_or_else(|e| panic!("{} is not under the repo root: {e}", file.display()));
            out.push((
                member.path.clone(),
                relative.to_string_lossy().replace('\\', "/"),
            ));
        }
    }
    out
}

/// One source file the coverage gate does not measure, and the exclusion that
/// takes it out.
#[derive(Debug)]
struct Dropped {
    /// The workspace member holding it, as the root `Cargo.toml` lists it.
    member: String,
    /// Repo-relative path — what a failure has to be able to name.
    file: String,
    /// The `_COV_IGNORE` alternative that matches it.
    exclusion: String,
}

/// Every member `src/` file `_COV_IGNORE` keeps out of the denominator.
///
/// Matched against a repo-relative path under a leading `/` rather than
/// against the absolute one llvm-cov sees. The absolute path here is this
/// crate's own directory plus `../..`, which spells `xtask/` — so the checkout
/// location would decide what counts as excluded, and the exclusion list would
/// appear to swallow the workspace.
fn dropped_sources(justfile: &str) -> Vec<Dropped> {
    let exclusions: Vec<(String, Regex)> = coverage_exclusions(justfile)
        .into_iter()
        .map(|alternative| {
            let compiled = Regex::new(&alternative).unwrap_or_else(|e| {
                panic!("`_COV_IGNORE` holds `{alternative}`, which is not a regex: {e}")
            });
            (alternative, compiled)
        })
        .collect();
    member_sources()
        .into_iter()
        .filter_map(|(member, file)| {
            let hit = exclusions
                .iter()
                .find(|(_, pattern)| pattern.is_match(&format!("/{file}")))?;
            Some(Dropped {
                member,
                file,
                exclusion: hit.0.clone(),
            })
        })
        .collect()
}

/// The members with at least one `src/` file out of the denominator. Partly is
/// enough: a crate is either wholly measured or it is one whose coverage
/// number answers for less than the crate.
fn dropped_members(justfile: &str) -> BTreeSet<String> {
    dropped_sources(justfile)
        .into_iter()
        .map(|dropped| crate_dir(&dropped.member).to_owned())
        .collect()
}

/// The binaries one member declares. An explicit `[[bin]]` names itself; a
/// crate with `src/main.rs` and no `[[bin]]` still gets one, named after the
/// package — and `CARGO_BIN_EXE_…` follows exactly that rule, which is what
/// makes the name the link between a manifest and a test that spawns it.
fn binary_names(member: &Member) -> Vec<String> {
    let declared: Vec<String> = table_pairs(&member.manifest, "bin")
        .into_iter()
        .filter(|&(key, _)| key == "name")
        .map(|(_, value)| value.trim_matches('"').to_owned())
        .collect();
    if !declared.is_empty() {
        return declared;
    }
    let implicit = repo_root().join(&member.path).join("src/main.rs").is_file();
    implicit
        .then(|| manifest_value(&member.manifest, "package", "name"))
        .flatten()
        .map(|name| name.trim_matches('"').to_owned())
        .into_iter()
        .collect()
}

/// One member's integration tests, read.
fn test_sources(member: &str) -> Vec<String> {
    let directory = repo_root().join(member).join("tests");
    if !directory.is_dir() {
        return Vec::new();
    }
    rust_files(&directory)
        .into_iter()
        .map(|file| {
            fs::read_to_string(&file).unwrap_or_else(|e| panic!("reading {}: {e}", file.display()))
        })
        .collect()
}

/// The members whose `src/` reaches crates.io.
fn published_members() -> BTreeSet<String> {
    workspace_members()
        .into_iter()
        .filter(|member| is_published(&member.manifest))
        .map(|member| crate_dir(&member.path).to_owned())
        .collect()
}

/// A recipe's body with backslash continuations folded into single lines.
/// `test-wasm` names its sub-command on one line and the crate it points at on
/// the next, and a per-line reader would see a `wasm-pack test` aimed at
/// nothing.
fn joined_body(justfile: &str, recipe: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut pending = String::new();
    for line in recipe_body(justfile, recipe) {
        let text = strip_comment(line).trim();
        if let Some(head) = text.strip_suffix('\\') {
            pending.push_str(head);
            pending.push(' ');
            continue;
        }
        pending.push_str(text);
        out.push(mem::take(&mut pending));
    }
    if !pending.is_empty() {
        out.push(pending);
    }
    out
}

/// The crate directories one command line hands `wasm-pack <sub>`. A directory
/// that exists is the test, rather than a `crates/` prefix: `--target bundler`
/// and `--out-dir pkg` sit in the same argument list and neither is a path
/// into this repo.
fn wasm_pack_targets(line: &str, sub: &str) -> Vec<String> {
    let tokens = shell_tokens(line);
    let mut out = Vec::new();
    for (at, token) in tokens.iter().enumerate() {
        if token != "wasm-pack" {
            continue;
        }
        let mut next = at + 1;
        while tokens.get(next).is_some_and(|token| token.starts_with('+')) {
            next += 1;
        }
        if tokens.get(next).map(String::as_str) != Some(sub) {
            continue;
        }
        out.extend(
            tokens[next + 1..]
                .iter()
                .filter(|token| !token.starts_with('-') && repo_root().join(token).is_dir())
                .cloned(),
        );
    }
    out
}

/// Every crate a gate hands `wasm-pack <sub>`, transitively through the
/// recipes it depends on — `playground-build` reaches `wasm-pack build` two
/// dependencies away, and a build is a build however far from the gate it is
/// written.
fn wasm_pack_paths(justfile: &str, sub: &str) -> BTreeSet<String> {
    let variables = plain_variables(justfile);
    let mut out = BTreeSet::new();
    for gate in recipes_in_group(justfile, "gate") {
        let mut pending = vec![gate];
        let mut seen = BTreeSet::new();
        while let Some(recipe) = pending.pop() {
            if !seen.insert(recipe.clone()) {
                continue;
            }
            pending.extend(header_dependencies(recipe_header(justfile, &recipe)));
            for line in joined_body(justfile, &recipe) {
                out.extend(wasm_pack_targets(&expand(&line, &variables), sub));
            }
        }
    }
    out
}

/// Every crate this repo compiles for wasm32 and excuses from the coverage
/// denominator without testing it there. Each one is a hole in the shape of
/// the exemption: llvm-cov instruments the host build, the exclusion says so,
/// and then no step reaches the code the exclusion is about.
fn coverage_debts(justfile: &str) -> Vec<String> {
    let excluded = dropped_members(justfile);
    let tested: BTreeSet<String> = wasm_pack_paths(justfile, "test")
        .iter()
        .map(|path| crate_dir(path).to_owned())
        .collect();
    wasm_pack_paths(justfile, "build")
        .into_iter()
        .map(|path| crate_dir(&path).to_owned())
        .filter(|path| excluded.contains(path) && !tested.contains(path))
        .collect()
}

#[test]
fn no_source_file_of_a_crate_this_repo_publishes_is_out_of_the_coverage_denominator() {
    let justfile = read("Justfile");
    let dropped = dropped_sources(&justfile);
    assert!(
        !dropped.is_empty(),
        "`_COV_IGNORE` matched no file under any member's `src/`. Every exclusion it carries \
         today names a crate in this workspace, so the likely answer is that this reader stopped \
         reading them — an escape it unescapes wrongly matches nothing, and a reader matching \
         nothing calls every exemption harmless. If the list really did shrink to entries that \
         reach no source, retarget this reader rather than leaving it passing over an empty set"
    );
    let published = published_members();
    let hidden: Vec<String> = dropped
        .iter()
        .filter(|dropped| published.contains(crate_dir(&dropped.member)))
        .map(|dropped| format!("{} (by `{}`)", dropped.file, dropped.exclusion))
        .collect();
    assert!(
        hidden.is_empty(),
        "{hidden:?} ships to crates.io and is outside the coverage denominator. No other gate \
         measures a `src/` file, so each of these is code the floor cannot see — and the floor \
         reads higher for their absence, which is the direction that hides the loss. Either \
         narrow the exclusion until it stops reaching published source, or drop it and \
         re-measure `_COV_FLOOR` over the wider denominator"
    );
}

#[test]
fn the_exemptions_that_stood_before_this_reader_hid_published_source() {
    // `_COV_IGNORE` as it was. Every assertion in this section passed on it:
    // the two entries doing the hiding carried regex punctuation, the reader
    // classified anything punctuated as "policy about regions", and policy was
    // what it declined to resolve. So the CLI entry points and the epub
    // serialisation pair were out of every gate in the repo, and the number
    // the floor is set from was measured without them.
    let before = concat!(
        "_COV_IGNORE := \"(target/|/main\\\\.rs$|xtask/\
         |aozora-flavored-markdown-test-support/|aozora-flavored-markdown-wasm/\
         |aozora-flavored-markdown-epub/src/(compose|package)\\\\.rs)\"\n",
        "\n",
        "[group('gate')]\n",
        "coverage:\n",
        "    cargo llvm-cov nextest --ignore-filename-regex '{{_COV_IGNORE}}'\n",
    );
    let published = published_members();
    let mut hidden: Vec<String> = dropped_sources(before)
        .into_iter()
        .filter(|dropped| published.contains(crate_dir(&dropped.member)))
        .map(|dropped| dropped.file)
        .collect();
    hidden.sort();
    assert_eq!(
        hidden,
        vec![
            "crates/aozora-flavored-markdown-cli/src/main.rs".to_owned(),
            "crates/aozora-flavored-markdown-epub-cli/src/main.rs".to_owned(),
            "crates/aozora-flavored-markdown-epub/src/compose.rs".to_owned(),
            "crates/aozora-flavored-markdown-epub/src/package.rs".to_owned(),
        ],
        "the reader no longer sees the defect it was written for"
    );
}

#[test]
fn the_coverage_gate_measures_every_crate_and_fails_under_the_floor_it_declares() {
    // What the reader above asserts is only worth its name if the gate reads
    // the whole workspace and fails on the number `_COV_FLOOR` holds. A gate
    // measuring one crate, or comparing against a literal while a variable
    // above it documents a different floor, would satisfy every other
    // assertion here while excusing far more than `_COV_IGNORE` names.
    let justfile = read("Justfile");
    let variables = plain_variables(&justfile);
    let floor = variables
        .get("_COV_FLOOR")
        .unwrap_or_else(|| panic!("no `_COV_FLOOR := \"…\"` in the Justfile"));
    let body: Vec<String> = joined_body(&justfile, "coverage")
        .iter()
        .map(|line| expand(line, &variables))
        .collect();
    assert!(
        recipes_in_group(&justfile, "gate").contains("coverage"),
        "the coverage recipe is not in the gate manifest, so nothing runs it"
    );
    assert!(
        body.iter().any(|line| line.contains("--workspace")),
        "the coverage gate measures something narrower than the workspace: {body:?}"
    );
    assert!(
        body.iter()
            .any(|line| line.contains(&format!("--fail-under-regions {floor}"))),
        "the coverage gate does not fail under `_COV_FLOOR` ({floor}); it runs {body:?}"
    );
}

#[test]
fn every_binary_this_repo_publishes_is_run_as_a_process_by_its_own_tests() {
    // What replaced the `/main\.rs$` exemption. The entry points are in the
    // denominator now on one condition — that a test spawns the binary, so
    // llvm-cov collects the child process — and that condition is a sentence
    // in the `Justfile` unless something reads it. The floor cannot: a `main`
    // reduced to `app::run()` is three regions in seven thousand, so tests
    // rewritten to call the crate in-process would leave the entry point
    // uncovered and every gate green.
    let mut checked = 0usize;
    let mut unspawned = Vec::new();
    for member in workspace_members() {
        if !is_published(&member.manifest) {
            continue;
        }
        let tests = test_sources(&member.path);
        for name in binary_names(&member) {
            checked += 1;
            let needle = format!("CARGO_BIN_EXE_{name}\"");
            if !tests.iter().any(|source| source.contains(&needle)) {
                unspawned.push(format!("{} ({name})", member.path));
            }
        }
    }
    assert!(
        checked >= 2,
        "this repo publishes two binaries and the reader found {checked}; it is not finding the \
         `[[bin]]` targets it is written to check"
    );
    assert!(
        unspawned.is_empty(),
        "{unspawned:?}: no test under the crate's own `tests/` names `CARGO_BIN_EXE_<bin>`, so \
         nothing runs the binary as a process. Its `main` is then measured by nothing while \
         sitting inside the coverage denominator — too few regions for the floor to notice, and \
         exactly the state the `main.rs` exemption used to describe honestly"
    );
}

#[test]
fn a_crate_excused_from_coverage_for_shipping_to_wasm_is_tested_on_wasm() {
    let justfile = read("Justfile");
    assert!(
        !wasm_pack_paths(&justfile, "build").is_empty(),
        "no gate reaches `wasm-pack build` any more; this reader must be retargeted, not left \
         passing over an empty set"
    );
    assert!(
        coverage_debts(&justfile).is_empty(),
        "{:?} is compiled for wasm32 by a gate and dropped from the coverage denominator, and \
         nothing runs its tests on wasm32. The exclusion buys silence rather than deferring to a \
         step: llvm-cov never sees the crate, and neither does anything else. Either point a \
         `[group('gate')]` recipe at `wasm-pack test <crate>`, or stop excluding it and let the \
         host build carry the floor",
        coverage_debts(&justfile)
    );
}

#[test]
fn the_arrangement_that_stood_before_test_wasm_reads_as_a_debt() {
    // The `Justfile` as it was: a gate compiles the crate for wasm32,
    // `_COV_IGNORE` drops it, and the comment above the exclusion defers to a
    // `wasm-pack test` step that is written nowhere. Every assertion in this
    // file passed on that, because every one of them is about a recipe that
    // exists rather than one that was promised.
    let before = concat!(
        "_COV_IGNORE := \"(target/|aozora-flavored-markdown-wasm/)\"\n",
        "\n",
        "[group('gate')]\n",
        "coverage:\n",
        "    cargo llvm-cov nextest --ignore-filename-regex '{{_COV_IGNORE}}'\n",
        "\n",
        "[group('gate')]\n",
        "playground-build: wasm-build\n",
        "    bun run build\n",
        "\n",
        "wasm-build:\n",
        "    bash -c 'wasm-pack build crates/aozora-flavored-markdown-wasm \\\n",
        "        --target bundler --out-dir pkg -- --locked'\n",
    );
    assert_eq!(
        coverage_debts(before),
        vec!["crates/aozora-flavored-markdown-wasm".to_owned()],
        "the reader no longer sees the defect it was written for"
    );

    let after = format!(
        "{before}\n\
         [group('gate')]\n\
         test-wasm:\n    \
         bash -c 'wasm-pack test --node \\\n        \
         crates/aozora-flavored-markdown-wasm --locked'\n"
    );
    assert!(
        coverage_debts(&after).is_empty(),
        "a gate that tests the crate on the target it ships to settles the exemption: {:?}",
        coverage_debts(&after)
    );
}

/// A `Justfile` holding one `_COV_IGNORE` and a coverage gate that uses it.
fn coverage_fixture(ignore: &str) -> String {
    format!(
        "_COV_IGNORE := \"({ignore})\"\n\
         \n\
         [group('gate')]\n\
         coverage:\n    \
         cargo llvm-cov nextest --ignore-filename-regex '{{{{_COV_IGNORE}}}}'\n"
    )
}

#[test]
fn an_exclusion_naming_files_inside_a_crate_is_those_files_and_the_crate_they_are_in() {
    // The shape that went unread. Answering "not a directory, so not mine"
    // left two published files unmeasured and unremarked; answering "the whole
    // crate" would have this file demand a wasm test for the epub generator.
    // It is neither: it is the files it matches, and the member each belongs
    // to — so the epub pair reads as published source going unmeasured, and
    // nothing about a wasm target follows from it.
    let justfile =
        coverage_fixture("aozora-flavored-markdown-epub/src/(compose|package)\\\\.rs|xtask/");
    let hit: Vec<String> = dropped_sources(&justfile)
        .into_iter()
        .filter(|dropped| dropped.member.ends_with("-epub"))
        .map(|dropped| dropped.file)
        .collect();
    assert_eq!(
        hit,
        vec![
            "crates/aozora-flavored-markdown-epub/src/compose.rs".to_owned(),
            "crates/aozora-flavored-markdown-epub/src/package.rs".to_owned(),
        ]
    );
    assert!(
        dropped_members(&justfile).contains("crates/aozora-flavored-markdown-epub"),
        "a pattern over a crate's files leaves that crate partly unmeasured, and a reader that \
         reported it as untouched would be the one that missed these two files"
    );
    assert!(
        coverage_debts(&justfile).is_empty(),
        "the epub generator ships to no wasm target, so no wasm test is owed for it"
    );
}

#[test]
fn a_crate_excused_by_a_pattern_over_its_files_is_excused_all_the_same() {
    // The same hole, aimed at the exemption this section was built for. The
    // wasm crate's entry could be spelled as a pattern over the files inside
    // it — the exact form `_COV_IGNORE` used for the epub pair — and the
    // shape-based reader answered `None` for anything punctuated, so the crate
    // read as measured, the debt read as settled, and `just test-wasm` could
    // have been deleted without a word from this file.
    let justfile = format!(
        "{}\n\
         [group('gate')]\n\
         playground-build: wasm-build\n    \
         bun run build\n\
         \n\
         wasm-build:\n    \
         bash -c 'wasm-pack build crates/aozora-flavored-markdown-wasm \\\n        \
         --target bundler --out-dir pkg -- --locked'\n",
        coverage_fixture("target/|aozora-flavored-markdown-wasm/src/.*\\\\.rs$")
    );
    assert_eq!(
        coverage_debts(&justfile),
        vec!["crates/aozora-flavored-markdown-wasm".to_owned()],
        "an exemption spelled as a pattern excuses the crate as thoroughly as one spelled as a \
         directory, and has to read as the same debt"
    );
}

#[test]
fn a_backslash_the_justfile_spells_twice_reaches_the_reader_once() {
    // `just` unescapes a `"…"` value before the shell ever sees it. A reader
    // holding the on-disk spelling would compile a regex for a literal
    // backslash, match nothing with it, and call every exemption written that
    // way harmless — silence that looks exactly like a clean workspace.
    assert_eq!(
        unescaped("src/(compose|package)\\\\.rs"),
        "src/(compose|package)\\.rs"
    );
    assert_eq!(unescaped("a\\\"b"), "a\"b");
    assert_eq!(unescaped("target/"), "target/");
    assert_eq!(
        coverage_exclusions(&coverage_fixture("/main\\\\.rs$")),
        vec!["/main\\.rs$".to_owned()]
    );
    assert!(
        !dropped_sources(&coverage_fixture("/main\\\\.rs$")).is_empty(),
        "the escaped form matched no `main.rs` in this workspace, so the reader is holding a \
         pattern the coverage gate never uses"
    );
}

#[test]
fn a_nested_group_is_one_exclusion_and_not_two() {
    assert_eq!(
        split_alternatives("target/|epub/src/(compose|package)\\.rs|xtask/"),
        vec!["target/", "epub/src/(compose|package)\\.rs", "xtask/"]
    );
    assert_eq!(split_alternatives("only/"), vec!["only/"]);
}

#[test]
fn a_crate_named_on_the_line_after_its_sub_command_is_still_the_target() {
    // `test-wasm` is written across two lines and `wasm-build` on one. The
    // reader has to answer the same for both, or a continuation would be a
    // place to hide an untested build.
    let justfile = read("Justfile");
    assert!(
        wasm_pack_paths(&justfile, "test").contains("crates/aozora-flavored-markdown-wasm"),
        "the `wasm-pack test` reader found {:?}",
        wasm_pack_paths(&justfile, "test")
    );
    // A flag's value is not a path, however much a bare word looks like one.
    assert_eq!(
        wasm_pack_targets(
            "wasm-pack build crates --target bundler --out-dir pkg",
            "build"
        ),
        vec!["crates".to_owned()],
        "a flag's value was read as the crate under build"
    );
    // And the sub-command has to match: a build is not a test run.
    assert!(wasm_pack_targets("wasm-pack build crates", "test").is_empty());
}

// ---------------------------------------------------------------------------
// the warning the flags cannot reach
// ---------------------------------------------------------------------------
//
// A fifth way a check can check nothing, and the only one of the five that was
// live on a published page rather than merely available. Everything in the doc
// section above measures RUSTDOCFLAGS, and RUSTDOCFLAGS is handed to rustdoc.
// `output filename collision` is CARGO's: two documentation units resolving to
// one `target/doc/<name>/`, the later of them overwriting the other's page,
// reported and exited 0 on. `-D warnings` never sees it, the probe that
// measures `-D warnings` never sees it, and neither could have.
//
// Both CLI binaries were named after their libraries — a binary's name is the
// shipped command, so `aozora-flavored-markdown` the bin and
// `aozora_flavored_markdown` the lib are one directory — and `docs.yml` copies
// `target/doc` to the API site that `[workspace.package] documentation` gives
// every reader of every published crate. The library's own URL served whichever
// unit finished last.
//
// The defect has two halves and they are read two ways. Whether this workspace
// still holds a pair that clashes is decidable from `cargo metadata`, which is
// cargo's own answer to "what will you document, and what is each one called".
// Whether the GATE would notice a pair introduced some other way is decidable
// from no file at all: it is answered by running a recipe's own script against
// a `cargo` that prints what cargo prints — captured from a workspace built to
// clash, so the gate is held to the tool's wording rather than to a wording
// written down twice.

/// The shell script a recipe hands `bash -c '…'`. The doc recipes wrap their
/// build in one because the collision check has to run after it and see what
/// it printed; it is also the only form in which that check can be exercised
/// here without building anything.
fn bash_script(line: &str) -> Option<&str> {
    let (_, after) = line.split_once("bash -c '")?;
    let end = after.rfind('\'')?;
    Some(&after[..end])
}

/// What a stubbed `cargo` says, and how it exits.
struct StubbedBuild<'a> {
    output: &'a str,
    status: i32,
}

/// Run a doc recipe's own script with `cargo` replaced by a stub that prints
/// `build.output` on STDERR — where cargo says everything it says, so a recipe
/// that reads its build back has to redirect that stream first — and exits
/// `build.status`. The answer is the exit code the recipe would give CI.
fn doc_script_exit(script: &str, build: &StubbedBuild<'_>) -> i32 {
    let dir = scratch("doc-script");
    let said = dir.join("cargo-said");
    fs::write(&said, build.output).unwrap_or_else(|e| panic!("writing {}: {e}", said.display()));
    let stub = dir.join("cargo");
    let status = build.status;
    let source = format!("#!/bin/sh\ncat '{}' >&2\nexit {status}\n", said.display());
    fs::write(&stub, source).unwrap_or_else(|e| panic!("writing {}: {e}", stub.display()));
    let marked = Command::new("chmod")
        .arg("+x")
        .arg(&stub)
        .status()
        .unwrap_or_else(|e| panic!("running chmod: {e}"));
    assert!(marked.success(), "chmod could not mark the stub executable");

    let path = format!("{}:{}", dir.display(), env::var("PATH").unwrap_or_default());
    let out = Command::new("bash")
        .arg("-c")
        .arg(script)
        .current_dir(&dir)
        // PATH so `cargo` is the stub and every other command is the real one;
        // TMPDIR so the `mktemp` the script runs lands in here and leaves with
        // it rather than in the image's `/tmp`.
        .env("PATH", path)
        .env("TMPDIR", &dir)
        .output()
        .unwrap_or_else(|e| panic!("running the recipe's own script: {e}"));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    drop(fs::remove_dir_all(&dir));
    // A script that cannot find a command exits non-zero for a reason that has
    // nothing to do with the check under test, and every "this fails" assertion
    // below would hold on it.
    assert!(
        !stderr.contains("command not found"),
        "the recipe's script could not find a command it runs:\n{stderr}"
    );
    out.status.code().unwrap_or(-1)
}

/// One `cargo doc` over a generated workspace, and what it left behind.
struct ProbeDocs {
    /// Did cargo exit 0? That answer is what puts a collision out of reach of
    /// every check that reads an exit status — which is every check this repo
    /// had.
    succeeded: bool,
    /// Everything cargo said, its two streams merged as the recipes merge them.
    output: String,
    /// Did the build write an entry page — `doc/index.html` — of its own?
    entry_page: bool,
    /// The page at the first library's own URL, whichever unit wrote it.
    library_page: Option<String>,
}

/// Document a workspace shaped like this one: two libraries, and a CLI crate
/// whose binary carries the first library's name.
///
/// Run rather than written down. "cargo warns and carries on", "the warning
/// reads like this" and "`doc = false` leaves the library's page in place" are
/// claims about cargo, and a file that spelled them out would go on asserting
/// them after they stopped being true — which is the failure this whole file
/// is about, one layer down.
fn probe_documentation(document_the_binary: bool) -> ProbeDocs {
    let dir = scratch("doc-collision");
    let doc = if document_the_binary { "true" } else { "false" };
    let files = [
        (
            "Cargo.toml",
            "[workspace]\nmembers = [\"alpha\", \"beta\", \"cli\"]\nresolver = \"2\"\n".to_owned(),
        ),
        (
            "alpha/Cargo.toml",
            probe_package("probe-alpha", "[lib]\npath = \"lib.rs\"\n"),
        ),
        (
            "alpha/lib.rs",
            "pub fn alpha_library_marker() {}\n".to_owned(),
        ),
        (
            "beta/Cargo.toml",
            probe_package("probe-beta", "[lib]\npath = \"lib.rs\"\n"),
        ),
        (
            "beta/lib.rs",
            "pub fn beta_library_marker() {}\n".to_owned(),
        ),
        (
            "cli/Cargo.toml",
            probe_package(
                "probe-alpha-cli",
                &format!("[[bin]]\nname = \"probe-alpha\"\npath = \"main.rs\"\ndoc = {doc}\n"),
            ),
        ),
        ("cli/main.rs", "fn main() {}\n".to_owned()),
    ];
    for (name, text) in files {
        let path = dir.join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)
                .unwrap_or_else(|e| panic!("creating {}: {e}", parent.display()));
        }
        fs::write(&path, text).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
    }

    let out = Command::new("cargo")
        // `--offline`, and no `--locked`: this workspace has no dependencies
        // and no lockfile of record, so there is nothing to fetch and nothing
        // to bind it to.
        .args(["doc", "--offline", "--no-deps", "--workspace"])
        .current_dir(&dir)
        // The dev image points CARGO_TARGET_DIR at a shared directory, which is
        // where THIS repo's `target/doc` lives. A probe that inherited it would
        // write its pages over the ones under test.
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running cargo doc: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where the toolchain is installed."
            )
        });
    let mut output = String::from_utf8_lossy(&out.stdout).into_owned();
    output.push_str(&String::from_utf8_lossy(&out.stderr));
    let docs = dir.join("target/doc");
    let probe = ProbeDocs {
        succeeded: out.status.success(),
        output,
        entry_page: docs.join("index.html").is_file(),
        library_page: fs::read_to_string(docs.join("probe_alpha/index.html")).ok(),
    };
    drop(fs::remove_dir_all(&dir));
    probe
}

/// One manifest of the probe workspace.
fn probe_package(name: &str, target: &str) -> String {
    format!("[package]\nname = \"{name}\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n{target}")
}

/// One target `cargo doc` will document.
#[derive(Debug, PartialEq, Eq)]
struct DocTarget {
    package: String,
    name: String,
    kind: String,
}

impl DocTarget {
    /// The target as a failure message has to name it.
    fn label(&self) -> String {
        let Self {
            package,
            name,
            kind,
        } = self;
        format!("the {kind} target `{name}` in {package}")
    }
}

/// Every target `cargo doc` documents, keyed by the directory rustdoc writes it
/// to. That directory is the target's name with `-` turned into `_`, so a
/// binary called `aozora-flavored-markdown` and a library called
/// `aozora_flavored_markdown` are one key — which is the whole defect.
///
/// The `doc` flag is cargo's own, and reading it is what keeps the key space
/// honest in both directions: a reader that took every target would collide on
/// the two `cli_integration` test binaries this workspace has always had, and a
/// reader that took none would find nothing to collide at all.
fn documented_targets(metadata: &str) -> BTreeMap<String, Vec<DocTarget>> {
    let parsed: Value = serde_json::from_str(metadata)
        .unwrap_or_else(|e| panic!("`cargo metadata` did not answer with JSON: {e}"));
    let packages = parsed["packages"]
        .as_array()
        .unwrap_or_else(|| panic!("`cargo metadata` answered with no `packages` array"));
    let mut out: BTreeMap<String, Vec<DocTarget>> = BTreeMap::new();
    for package in packages {
        let package_name = json_string(package, "name");
        for target in package["targets"]
            .as_array()
            .unwrap_or_else(|| panic!("`{package_name}` came back with no `targets` array"))
        {
            let name = json_string(target, "name");
            let documented = target["doc"].as_bool().unwrap_or_else(|| {
                panic!(
                    "`cargo metadata` no longer says whether it documents `{name}`. Without that \
                     flag every target reads the same way, and both answers are wrong: one \
                     collides the test binaries, the other collides nothing ever again."
                )
            });
            if !documented {
                continue;
            }
            let kind = target["kind"]
                .as_array()
                .and_then(|kinds| kinds.first())
                .and_then(Value::as_str)
                .unwrap_or_else(|| panic!("`{name}` came back with no `kind`"))
                .to_owned();
            out.entry(name.replace('-', "_"))
                .or_default()
                .push(DocTarget {
                    package: package_name.clone(),
                    name,
                    kind,
                });
        }
    }
    out
}

/// One string field of a `cargo metadata` object.
fn json_string(value: &Value, key: &str) -> String {
    value[key]
        .as_str()
        .unwrap_or_else(|| panic!("`cargo metadata` answered with no `{key}`: {value}"))
        .to_owned()
}

/// The rustdoc output directories more than one documented target writes.
fn colliding_documentation(metadata: &str) -> Vec<String> {
    documented_targets(metadata)
        .into_iter()
        .filter(|(_, writers)| writers.len() > 1)
        .map(|(output, writers)| {
            let names: Vec<String> = writers.iter().map(DocTarget::label).collect();
            format!("target/doc/{output}/ is written by {}", names.join(" and "))
        })
        .collect()
}

/// What cargo says the targets of this workspace are.
///
/// Asked of cargo rather than derived from the manifests: which targets exist
/// and which of them get documented is cargo's rule, not a rule — an explicit
/// `[[bin]]`, an inferred `src/main.rs`, a `[lib] name`, a `doc = false` — and
/// a second model of it here would be a second answer waiting to disagree with
/// the one that builds the site.
fn workspace_metadata() -> String {
    let out = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running cargo metadata: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where the toolchain is installed."
            )
        });
    assert!(
        out.status.success(),
        "`cargo metadata` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn every_doc_gate_fails_on_the_collision_cargo_reports() {
    // The widening of `every_rustdoc_build_this_repo_drives_denies_warnings`
    // from rustdoc's warnings to the BUILD's. That test, and the probe that
    // measures its flags, are both about lints rustdoc emits; the warning that
    // decided what the API site served came from cargo, on the same stderr, and
    // passed both.
    //
    // Over every documentation build the Justfile defines, for the reason the
    // deny assertion is: a third recipe added without the check is this hole
    // reopened, and naming today's two would not see it.
    let justfile = read("Justfile");
    let builds = doc_builds(&justfile);
    assert!(
        !builds.is_empty(),
        "the Justfile came out with no rustdoc build at all; the reader is not finding \
         `cargo doc` in a recipe body"
    );

    let clash = probe_documentation(true);
    assert!(
        clash.output.contains("output filename collision"),
        "the probe workspace no longer makes cargo report a collision, so this test would pass \
         over any recipe at all. Retarget it rather than leaving it green:\n{}",
        clash.output
    );
    let clean = probe_documentation(false);

    for build in &builds {
        let script = bash_script(&build.line).unwrap_or_else(|| {
            panic!(
                "`just {}` no longer hands its build to `bash -c '…'`, so this test cannot run \
                 what the gate runs. Teach it the new shape:\n      {}",
                build.recipe,
                build.line.trim()
            )
        });
        // The control. Without it, "the recipe exits non-zero" would hold just
        // as well on a recipe that fails for every build there is.
        assert_eq!(
            doc_script_exit(
                script,
                &StubbedBuild {
                    output: &clean.output,
                    status: 0
                }
            ),
            0,
            "`just {}` fails on a documentation build that produced no warning at all:\n      {}",
            build.recipe,
            build.line.trim()
        );
        assert_ne!(
            doc_script_exit(
                script,
                &StubbedBuild {
                    output: &clash.output,
                    status: 0
                }
            ),
            0,
            "`just {}` exits 0 on the build cargo reported an output filename collision over. \
             The two units wrote one `target/doc/<name>/`, so the page that survives is whichever \
             finished last — and `docs.yml` copies that directory to the API site. RUSTDOCFLAGS \
             cannot reach this: cargo said it, not rustdoc.\n      {}",
            build.recipe,
            build.line.trim()
        );
    }
}

#[test]
fn reading_the_builds_output_did_not_cost_a_doc_gate_its_exit_status() {
    // The regression the fix itself could introduce. Reading what the build
    // printed means running it in a pipeline, and a pipeline reports the exit
    // status of its LAST command — `tee`, which always succeeds. So a recipe
    // that grew the collision check without `set -o pipefail`, or that ended on
    // the check rather than on the build's own status, would swallow every
    // rustdoc failure the section above exists to produce: `-D warnings` still
    // handed over, rustdoc still failing, the gate still green.
    let justfile = read("Justfile");
    let builds = doc_builds(&justfile);
    assert!(!builds.is_empty(), "no rustdoc build found in the Justfile");
    for build in &builds {
        let Some(script) = bash_script(&build.line) else {
            panic!(
                "`just {}` no longer hands its build to `bash -c '…'`",
                build.recipe
            )
        };
        assert_ne!(
            doc_script_exit(
                script,
                &StubbedBuild {
                    output: "error: unclosed HTML tag `div`\nerror: aborting due to 1 error\n",
                    status: 1,
                }
            ),
            0,
            "`just {}` exits 0 on a documentation build that FAILED. Whatever it does with the \
             build's output, the build's own status has to survive it.\n      {}",
            build.recipe,
            build.line.trim()
        );
    }
}

#[test]
fn no_two_targets_this_workspace_documents_write_one_rustdoc_directory() {
    // The state, rather than the gate that would catch it changing. Two reasons
    // it is worth asserting separately from the recipes above: the recipes only
    // answer while something runs them, and a clash does not always survive to
    // be reported — two rustdoc units racing on one output directory can also
    // remove a file the other is still writing, which fails the doc build with
    // a path in the message and no mention of the cause.
    let metadata = workspace_metadata();
    let documented = documented_targets(&metadata);
    assert!(
        documented.len() >= 4,
        "cargo came back documenting {documented:?}; the reader is not finding the targets"
    );
    let clashes = colliding_documentation(&metadata);
    assert!(
        clashes.is_empty(),
        "{clashes:?}\n\
         One `cargo doc --workspace` pass writes them all into one `target/doc`, so the later unit \
         overwrites the earlier and the page a URL serves is whichever finished last. `docs.yml` \
         copies that directory to the API site `[workspace.package] documentation` sends every \
         reader to. Give whichever target is not the published page `doc = false` in its manifest."
    );
}

/// `cargo metadata` as it answered before the two `[[bin]] doc = false` lines:
/// each CLI's binary is named after the library it drives, because that name is
/// the shipped command.
const METADATA_BEFORE: &str = r#"{"packages":[
  {"name":"aozora-flavored-markdown","targets":[
    {"name":"aozora_flavored_markdown","kind":["lib"],"doc":true},
    {"name":"ast-walk","kind":["example"],"doc":false}]},
  {"name":"aozora-flavored-markdown-cli","targets":[
    {"name":"aozora-flavored-markdown","kind":["bin"],"doc":true},
    {"name":"cli_integration","kind":["test"],"doc":false}]},
  {"name":"aozora-flavored-markdown-epub","targets":[
    {"name":"aozora_flavored_markdown_epub","kind":["lib"],"doc":true}]},
  {"name":"aozora-flavored-markdown-epub-cli","targets":[
    {"name":"aozora-flavored-markdown-epub","kind":["bin"],"doc":true},
    {"name":"cli_integration","kind":["test"],"doc":false}]}]}"#;

#[test]
fn the_pair_this_repo_shipped_with_is_what_the_collision_reader_reports() {
    assert_eq!(
        colliding_documentation(METADATA_BEFORE),
        vec![
            "target/doc/aozora_flavored_markdown/ is written by the lib target \
             `aozora_flavored_markdown` in aozora-flavored-markdown and the bin target \
             `aozora-flavored-markdown` in aozora-flavored-markdown-cli"
                .to_owned(),
            "target/doc/aozora_flavored_markdown_epub/ is written by the lib target \
             `aozora_flavored_markdown_epub` in aozora-flavored-markdown-epub and the bin target \
             `aozora-flavored-markdown-epub` in aozora-flavored-markdown-epub-cli"
                .to_owned(),
        ],
        "the reader no longer sees the defect it was written for"
    );
    // And the trap inside the same answer: both CLI crates carry a test target
    // called `cli_integration`. They share a name and share nothing else — cargo
    // documents neither — so a reader that skipped the `doc` flag would report a
    // clash that has never existed and miss the two that did.
    assert!(
        !colliding_documentation(METADATA_BEFORE)
            .iter()
            .any(|clash| clash.contains("cli_integration")),
        "two targets cargo does not document were read as writing one page"
    );
}

#[test]
fn cargo_calls_one_page_overwriting_another_a_warning_and_exits_zero() {
    // Why no gate here could have caught this by watching an exit status, and
    // why `doc = false` is the remedy the manifests apply. Which of the two
    // pages survives is a race and is not asserted; that it is a race is the
    // defect.
    let clash = probe_documentation(true);
    assert!(
        clash.succeeded,
        "cargo now FAILS a build whose targets collide. That is the stronger behaviour and the \
         recipes' check becomes redundant — but check it deliberately rather than by this test \
         going quiet:\n{}",
        clash.output
    );
    assert!(
        clash.output.contains("output filename collision"),
        "the probe workspace no longer reproduces the collision; retarget it:\n{}",
        clash.output
    );

    let settled = probe_documentation(false);
    assert!(
        settled.succeeded && !settled.output.contains("output filename collision"),
        "`doc = false` on the binary did not settle the clash:\n{}",
        settled.output
    );
    let page = settled.library_page.unwrap_or_else(|| {
        panic!(
            "nothing was written at the library's own URL:\n{}",
            settled.output
        )
    });
    assert!(
        page.contains("alpha_library_marker"),
        "with the binary undocumented, the page at the library's URL is still not the library's"
    );
}

#[test]
fn a_documentation_build_of_this_shape_writes_no_entry_page_of_its_own() {
    // What makes the assertion below about `api/` load-bearing. rustdoc writes
    // an index over several crates only behind `--enable-index-page`, which is
    // nightly-only and therefore not available here, so `target/doc` has no
    // `index.html` — and the directory the site serves at the documentation URL
    // has no page at all unless the assembly writes one.
    let docs = probe_documentation(false);
    assert!(
        docs.library_page.is_some(),
        "the probe documented nothing, so it cannot answer this:\n{}",
        docs.output
    );
    assert!(
        !docs.entry_page,
        "`cargo doc` now writes a `target/doc/index.html` of its own. The site assembly writes one \
         too and the copy would land on top of it — decide which page the API root serves rather \
         than shipping both."
    );
}

// --- the entry point the site advertises ------------------------------------

/// A URL the workspace manifest advertises.
fn workspace_url(root: &str, key: &str) -> String {
    let value = manifest_value(root, "workspace.package", key)
        .unwrap_or_else(|| panic!("[workspace.package] no longer sets `{key}`"));
    value.trim().trim_matches('"').to_owned()
}

/// Where the docs workflow copies the documentation build into the site it
/// assembles, as a site-relative directory.
fn documentation_root(workflow: &str) -> String {
    workflow
        .lines()
        .map(strip_comment)
        .filter(|body| body.contains("target/doc"))
        .find_map(|body| {
            body.split_whitespace()
                .find_map(|word| word.strip_prefix("site/"))
                .map(str::to_owned)
        })
        .unwrap_or_else(|| {
            panic!("docs.yml no longer copies `target/doc` into the site it uploads")
        })
}

/// The pages the docs workflow writes into that site, as the site-relative path
/// of each → the URL its `<meta http-equiv="refresh">` sends a reader to.
fn site_redirects(workflow: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let mut destination: Option<String> = None;
    for line in workflow.lines() {
        let body = strip_comment(line);
        if let Some((_, after)) = body.split_once("url=") {
            destination = after.split(['"', '\'', '>', ' ']).next().map(str::to_owned);
        }
        let Some((_, after)) = body.split_once("> site/") else {
            continue;
        };
        let Some(page) = after.split_whitespace().next() else {
            continue;
        };
        if let Some(url) = destination.take() {
            out.insert(page.to_owned(), url);
        }
    }
    out
}

/// The file a site path is served by. A path naming a directory is served by
/// the `index.html` inside it — which is how `api/` could 404 while every page
/// under it was present and correct.
fn served_file(path: &str) -> String {
    if path.is_empty() || path.ends_with('/') {
        format!("{path}index.html")
    } else {
        path.to_owned()
    }
}

/// The trail a reader walks from a site path, hop by hop, through the redirects
/// the assembly writes. The last entry is where they land.
fn redirect_trail(redirects: &BTreeMap<String, String>, start: &str) -> Vec<String> {
    let mut at = served_file(start);
    let mut trail = vec![at.clone()];
    while let Some(url) = redirects.get(&at) {
        assert!(
            !url.starts_with('/') && !url.contains("://") && !url.contains(".."),
            "`{at}` redirects to `{url}`; this reader follows site-relative hops only"
        );
        let next = {
            let parent = at.rfind('/').map_or("", |slash| &at[..=slash]);
            served_file(&format!("{parent}{url}"))
        };
        assert!(
            !trail.contains(&next),
            "the site's redirects loop: {trail:?} → {next}"
        );
        at = next;
        trail.push(at.clone());
    }
    trail
}

/// The names of the packages that reach crates.io.
fn published_package_names() -> BTreeSet<String> {
    workspace_members()
        .iter()
        .filter(|member| is_published(&member.manifest))
        .filter_map(|member| {
            manifest_value(&member.manifest, "package", "name")
                .map(|name| name.trim_matches('"').to_owned())
        })
        .collect()
}

#[test]
fn the_documentation_url_this_workspace_advertises_lands_on_the_library() {
    // The acceptance the collision is about, stated as a property rather than
    // as a click: a reader who opens the URL every published crate advertises
    // reaches the LIBRARY's page, and reaches it because exactly one documented
    // target writes the directory they land in.
    //
    // Both halves have been false on the live site. The directory was written
    // by two targets, which is the rest of this section; and `api/` itself had
    // no page — cargo writes no entry page for a workspace of several crates
    // (measured next door), so the URL printed on every crates.io page here,
    // and the one the site root's own redirect points at, resolved to nothing.
    let root = read("Cargo.toml");
    let site = workspace_url(&root, "homepage");
    let advertised = workspace_url(&root, "documentation");
    let path = advertised.strip_prefix(&site).unwrap_or_else(|| {
        panic!(
            "`documentation` is `{advertised}` and the site docs.yml assembles is `{site}`. A \
             documentation URL outside that site is a page nothing in this repo can answer for."
        )
    });

    let workflow = read(".github/workflows/docs.yml");
    let redirects = site_redirects(&workflow);
    assert!(
        !redirects.is_empty(),
        "docs.yml writes no page of its own into the site; the reader is not finding the assembly"
    );
    let root_directory = documentation_root(&workflow);
    let documented = documented_targets(&workspace_metadata());

    for start in ["", path] {
        let trail = redirect_trail(&redirects, start);
        let landed = trail.last().expect("a trail holds where it started");
        let directory = landed
            .strip_prefix(&root_directory)
            .and_then(|rest| rest.strip_suffix("/index.html"))
            .unwrap_or_else(|| {
                panic!(
                    "a reader opening `{site}{start}` walks {trail:?} and stops at `{landed}`, \
                     which is not a crate page under `{root_directory}`. Every hop has to be a page \
                     something writes, and `cargo doc` writes none at the root of a multi-crate \
                     build: a directory with no `index.html` and nothing redirecting out of it is \
                     a 404."
                )
            });
        let writers = documented.get(directory).unwrap_or_else(|| {
            panic!(
                "`{landed}` is where the documentation URL lands, and no target this workspace \
                 documents writes `{directory}/`. Renaming the crate, or dropping it out of the \
                 doc build, moves the page and leaves the redirect pointing at nothing."
            )
        });
        assert_eq!(
            writers.len(),
            1,
            "the documentation URL lands on a page {} targets write: {writers:?}. Which one a \
             reader gets is whichever finished last.",
            writers.len()
        );
        assert_eq!(
            writers[0].kind,
            "lib",
            "the documentation URL lands on {}. The published API is the library's page.",
            writers[0].label()
        );
        assert!(
            published_package_names().contains(&writers[0].package),
            "the documentation URL lands on {}, and that crate does not reach crates.io — the \
             readers this URL is printed for cannot depend on it.",
            writers[0].label()
        );
    }
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn a_script_a_recipe_hands_bash_is_read_whole() {
    assert_eq!(
        bash_script("docker compose run --rm dev bash -c 'a; b'"),
        Some("a; b")
    );
    assert_eq!(bash_script("    cargo doc --locked --workspace"), None);
    // To the LAST quote. The recipes' scripts hold `'`-free double-quoted
    // arguments, and a reader stopping at the first quote inside one would run
    // half a gate and call it green.
    assert_eq!(
        bash_script("x bash -c 'cargo doc | tee f; grep -qF \"x\" f && exit 1'"),
        Some("cargo doc | tee f; grep -qF \"x\" f && exit 1")
    );
}

#[test]
fn a_single_quoted_recipe_variable_resolves_like_a_double_quoted_one() {
    let justfile = concat!(
        "_A := \"x\"\n",
        "_B := 'y'\n",
        "_dev := if _in == \"1\" { \"\" } else { \"docker compose run --rm dev\" }\n",
    );
    let variables = plain_variables(justfile);
    assert_eq!(variables.get("_A").map(String::as_str), Some("x"));
    assert_eq!(
        variables.get("_B").map(String::as_str),
        Some("y"),
        "a `'…'` value went unresolved, so a recipe interpolating it would read as unguarded"
    );
    assert!(
        !variables.contains_key("_dev"),
        "an `if` expression was resolved to one of its branches"
    );
}

#[test]
fn a_redirect_the_site_writes_is_followed_and_one_it_does_not_ends_the_trail() {
    let workflow = concat!(
        "jobs:\n",
        "  build:\n",
        "    steps:\n",
        "      # url=not-a-redirect/ in a comment, and > site/not-a-page.html\n",
        "      - run: |\n",
        "          mkdir -p site/api\n",
        "          cp -r target/doc/. site/api/\n",
        "          printf '%s\\n' \\\n",
        "            '<!doctype html><meta http-equiv=\"refresh\" content=\"0; url=api/\">' \\\n",
        "            > site/index.html\n",
        "          printf '%s\\n' \\\n",
        "            '<!doctype html><meta http-equiv=\"refresh\" content=\"0; url=lib_crate/\">' \\\n",
        "            > site/api/index.html\n",
    );
    assert_eq!(documentation_root(workflow), "api/");
    let redirects = site_redirects(workflow);
    assert_eq!(
        redirect_trail(&redirects, ""),
        vec![
            "index.html".to_owned(),
            "api/index.html".to_owned(),
            "api/lib_crate/index.html".to_owned(),
        ],
        "the walk from the site root did not follow both hops to a crate page"
    );
    assert!(
        !redirects.contains_key("not-a-page.html"),
        "a `> site/…` written in a comment counted as a page the site serves"
    );
    // The state this file was written on: `api/` with no page of its own is
    // where the walk stops, one hop short of any crate.
    let without = BTreeMap::from([("index.html".to_owned(), "api/".to_owned())]);
    assert_eq!(
        redirect_trail(&without, ""),
        vec!["index.html".to_owned(), "api/index.html".to_owned()],
        "a hop to a page nothing writes has to end the trail there, not silently continue"
    );
}

// ---------------------------------------------------------------------------
// the fuzz builds
// ---------------------------------------------------------------------------
//
// Everything above reads a `[group('gate')]` recipe. The `[group('fuzz')]`
// ones were read by nothing, and one of them — `fuzz-all-deep` — describes
// itself as "the gate before tagging a release". It had never run. Neither had
// any of the others: `cargo fuzz` takes `--target` from the platform its OWN
// binary was built for, the Dockerfile binstalls the upstream musl release
// asset, and no image here has ever had a musl target. So `just fuzz-quick
// parse_render` on a clean checkout printed two errors about neither this
// workspace nor its fuzz targets — `sanitizer is incompatible with statically
// linked libc`, then `can't find crate for core` — and the release pre-flight
// was a gate that could not start (DEV-230).
//
// A flag is what fixed it, so a flag is what the next reader deletes as a
// redundant default. The three assertions below are the two rustc facts that
// make it load-bearing, plus the one that says it is there at all.

/// The `cargo fuzz` sub-commands that compile the harness, and so build for a
/// triple. `add`, `init` and `list` do not.
const FUZZ_BUILDING: &[&str] = &["build", "cmin", "coverage", "run", "tmin"];

/// What a fuzz build with no `--target` builds for: cargo-fuzz's own default,
/// as `cargo fuzz run --help` prints it (0.13.2: `[default:
/// x86_64-unknown-linux-musl]`).
///
/// It is a fact about the cargo-fuzz binary rather than about this workspace
/// or this image — cargo-fuzz derives it from the platform it was itself
/// compiled for. That is what makes an unstated target unstated in the sense
/// that matters: nothing in this repo chose it, and nothing in this repo would
/// notice it changing.
const CARGO_FUZZ_DEFAULT_TRIPLE: &str = "x86_64-unknown-linux-musl";

/// A `cargo fuzz` build the `Justfile` drives: the recipe that runs it, the
/// line, and the triple it names — `None` when it names none and takes
/// cargo-fuzz's default.
struct FuzzBuild {
    recipe: String,
    line: String,
    triple: Option<String>,
}

impl FuzzBuild {
    /// The triple this build actually compiles for. A build that names none
    /// still has one, and that is the point: the two assertions that measure
    /// what a triple can do have to be asked of the default as well, or they
    /// go quiet on exactly the arrangement they exist to reject.
    fn effective_triple(&self) -> &str {
        self.triple.as_deref().unwrap_or(CARGO_FUZZ_DEFAULT_TRIPLE)
    }
}

/// The position of the sub-command in `cargo [+toolchain] fuzz <sub>`, when
/// these tokens run one that compiles.
fn fuzz_build_at(tokens: &[String]) -> Option<usize> {
    for (at, token) in tokens.iter().enumerate() {
        if token != "cargo" {
            continue;
        }
        // `cargo +nightly fuzz run` — the toolchain prefix is not the
        // sub-command, the same way `tool_commands` reads it.
        let mut next = at + 1;
        while tokens.get(next).is_some_and(|token| token.starts_with('+')) {
            next += 1;
        }
        if tokens.get(next).map(String::as_str) != Some("fuzz") {
            continue;
        }
        if tokens
            .get(next + 1)
            .is_some_and(|sub| FUZZ_BUILDING.contains(&sub.as_str()))
        {
            return Some(next + 1);
        }
    }
    None
}

/// The triple these tokens name, in either spelling cargo-fuzz takes, and only
/// where cargo-fuzz can see it: what follows a bare `--` is handed to
/// libFuzzer, which has no `--target` at all. `--target-dir` is a different
/// flag and is not this one.
fn target_triple(tokens: &[String]) -> Option<String> {
    let own = tokens.split(|token| token == "--").next().unwrap_or(&[]);
    for (at, token) in own.iter().enumerate() {
        if let Some(triple) = token.strip_prefix("--target=") {
            return Some(triple.to_owned());
        }
        if token == "--target" {
            return own.get(at + 1).cloned();
        }
    }
    None
}

/// Every `cargo fuzz` build in the `Justfile`, attributed to its recipe.
fn fuzz_builds(justfile: &str) -> Vec<FuzzBuild> {
    let mut out = Vec::new();
    for (recipe, expanded) in expanded_recipe_lines(justfile) {
        let tokens = shell_tokens(&expanded);
        let Some(at) = fuzz_build_at(&tokens) else {
            continue;
        };
        out.push(FuzzBuild {
            recipe,
            triple: target_triple(&tokens[at..]),
            line: expanded,
        });
    }
    out
}

/// Every triple the `Justfile`'s fuzz recipes compile for, mapped to a recipe
/// that compiles for it. Keyed by triple because the assertions below shell
/// out to rustc, and five recipes naming one triple is one question.
fn fuzz_triples(justfile: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for build in fuzz_builds(justfile) {
        out.entry(build.effective_triple().to_owned())
            .or_insert(build.recipe);
    }
    out
}

/// A crate that needs nothing but a target's `std` to compile.
const TRIPLE_PROBE_SOURCE: &str = "pub fn probe() {}\n";

/// Can this toolchain compile for `triple`? `Err` carries rustc's own words.
///
/// Run rather than read: "the target is installed" is a fact about the image,
/// and a `--target` asserted against a list written here would pass on an
/// image that has never had it — which is the entire defect, one level up.
fn rustc_can_build_for(triple: &str) -> Result<(), String> {
    let dir = scratch("triple-probe");
    let source = dir.join("probe.rs");
    fs::write(&source, TRIPLE_PROBE_SOURCE)
        .unwrap_or_else(|e| panic!("writing {}: {e}", source.display()));
    let out = Command::new("rustc")
        .arg(&source)
        .args(["--crate-type", "lib", "--emit", "metadata"])
        .arg("--out-dir")
        .arg(&dir)
        .args(["--target", triple])
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running rustc: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where the toolchain is installed."
            )
        });
    let accepted = out.status.success();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    drop(fs::remove_dir_all(&dir));
    if accepted { Ok(()) } else { Err(stderr) }
}

/// Does a build for `triple` link libc statically? `--print cfg` is where
/// rustc answers, and the answer is the whole of the first error: cargo-fuzz
/// builds with a sanitizer by default (`-s, --sanitizer [default:
/// address]`), and rustc refuses a sanitizer on a statically linked libc.
fn links_libc_statically(triple: &str) -> bool {
    let out = Command::new("rustc")
        .args(["--print", "cfg", "--target", triple])
        .output()
        .unwrap_or_else(|e| panic!("running rustc: {e}"));
    assert!(
        out.status.success(),
        "rustc does not know the target `{triple}`:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .any(|line| line.trim() == r#"target_feature="crt-static""#)
}

#[test]
fn every_fuzz_build_names_the_triple_it_builds_for() {
    // The state this section was written on: five invocations, no `--target`
    // between them, so every one of them built for whatever platform the
    // installed cargo-fuzz binary happened to come from.
    //
    // Stated over every fuzz build the file defines rather than over the
    // recipes by name, for the reason the rustdoc assertion is: a sixth recipe
    // added without the flag is this hole reopened, and a list of today's five
    // would not see it.
    let justfile = read("Justfile");
    let builds = fuzz_builds(&justfile);
    assert!(
        builds.len() >= 5,
        "the Justfile came out driving {} `cargo fuzz` build(s); the reader is not finding them \
         in the recipe bodies",
        builds.len()
    );

    for build in &builds {
        assert!(
            build.triple.is_some(),
            "`just {}` runs cargo-fuzz without `--target`, so it builds for `{CARGO_FUZZ_DEFAULT_TRIPLE}` \
             — the platform the cargo-fuzz BINARY was built for, which is a property of the \
             release asset the Dockerfile binstalls and of nothing this repo controls:\n      {}",
            build.recipe,
            build.line.trim()
        );
    }
}

#[test]
fn every_fuzz_build_compiles_for_a_triple_this_image_has() {
    // The second of the two errors the recipes printed, as a property. Asked
    // of the EFFECTIVE triple, so a recipe that names none is asked about
    // cargo-fuzz's default rather than skipped — being unstated is what put
    // the build on an uninstalled target in the first place.
    let justfile = read("Justfile");
    let triples = fuzz_triples(&justfile);
    assert!(
        !triples.is_empty(),
        "the Justfile came out driving no `cargo fuzz` build at all; the reader is not finding \
         them in the recipe bodies"
    );

    for (triple, recipe) in &triples {
        if let Err(stderr) = rustc_can_build_for(triple) {
            panic!(
                "`just {recipe}` compiles for `{triple}` and this toolchain cannot:\n{stderr}\
                 A target with no `std` installed fails before one fuzz target is built. Nothing \
                 in this image installs a target other than the host's, so the triple has to be \
                 that one — stated, because cargo-fuzz's own default is not it."
            );
        }
    }

    // The control, and the measurement the flag is FOR. Without it this test
    // says only that some triple compiles; with it, it says the one cargo-fuzz
    // picks unaided does not.
    assert!(
        rustc_can_build_for(CARGO_FUZZ_DEFAULT_TRIPLE).is_err(),
        "`{CARGO_FUZZ_DEFAULT_TRIPLE}` — cargo-fuzz's default — now compiles here, so the probe \
         above no longer tells an installed target from a missing one. Installing it is still not \
         a repair: the crt-static assertion below is why a musl fuzz build cannot run a sanitizer \
         either."
    );
}

#[test]
fn no_fuzz_build_compiles_for_a_statically_linked_libc() {
    // The first error, and the half that outlives any image change: a target
    // whose libc is static is a target AddressSanitizer refuses, whatever is
    // installed. It is why installing `x86_64-unknown-linux-musl` into the
    // fuzz stage — the repair DEV-230 offered first, on the merit of keeping
    // cargo-fuzz's static linking — repairs nothing: it trades the missing-std
    // error for this one, and getting past this one means passing
    // `-C target-feature=-crt-static`, i.e. handing back the static linking
    // that was the whole argument for going there.
    let justfile = read("Justfile");
    let triples = fuzz_triples(&justfile);
    assert!(
        !triples.is_empty(),
        "the Justfile came out driving no `cargo fuzz` build at all; the reader is not finding \
         them in the recipe bodies"
    );

    for (triple, recipe) in &triples {
        assert!(
            !links_libc_statically(triple),
            "`just {recipe}` compiles for `{triple}`, whose libc is statically linked. cargo-fuzz \
             builds with AddressSanitizer by default and rustc rejects a sanitizer on a static \
             libc, so this fails before one fuzz target is built."
        );
    }

    // The control: the default is in that state, which is why it never ran.
    assert!(
        links_libc_statically(CARGO_FUZZ_DEFAULT_TRIPLE),
        "`{CARGO_FUZZ_DEFAULT_TRIPLE}` no longer reports `crt-static`, so the account this test \
         and the `_FUZZ_TRIPLE` comment both give of why the default could not work is out of date"
    );
}

// ---------------------------------------------------------------------------
// the fuzz targets, wherever they are listed
// ---------------------------------------------------------------------------

/// The fuzz targets the fuzz crate registers. `fuzz/Cargo.toml` is the
/// registry: a `[[bin]]` there is what `cargo fuzz run <name>` resolves.
fn registered_fuzz_targets() -> BTreeSet<String> {
    let manifest = read("crates/aozora-flavored-markdown/fuzz/Cargo.toml");
    table_pairs(&manifest, "bin")
        .into_iter()
        .filter(|&(key, _)| key == "name")
        .flat_map(|(_, value)| quoted_items(value))
        .collect()
}

/// Every word a recipe's body says, comments off.
///
/// The sweeps are read for what they do NOT say now: each derives its target
/// list from `just _fuzz-targets` (`cargo fuzz list`), so a target name
/// spelled anywhere in one of them is a hand-written copy of that list growing
/// back.
fn recipe_words(justfile: &str, recipe: &str) -> BTreeSet<String> {
    recipe_body(justfile, recipe)
        .iter()
        .flat_map(|line| words(strip_comment(line)).map(str::to_owned))
        .collect()
}

/// The `[group('fuzz')]` recipes that take no argument, i.e. the ones that act
/// on the whole target set rather than on the one target a caller named.
/// `fuzz-quick TARGET` is asked nothing below; `fuzz-all-quick` is asked
/// everything.
///
/// Derived rather than listed, because a list of which recipes must derive
/// their targets is the same hand-written listing one level up: writing
/// `fuzz-all-medium` with the four names in it would satisfy a check that only
/// knew about today's three sweeps.
fn whole_set_fuzz_recipes(justfile: &str) -> BTreeSet<String> {
    recipes_in_group(justfile, "fuzz")
        .into_iter()
        .filter(|recipe| recipe_parameters(recipe_header(justfile, recipe)).is_empty())
        .collect()
}

/// The sweeps: whole-set recipes that ask `just _fuzz-targets` what the set is.
fn derived_sweeps(justfile: &str) -> BTreeSet<String> {
    whole_set_fuzz_recipes(justfile)
        .into_iter()
        .filter(|recipe| runs_recipe(&recipe_body(justfile, recipe), "_fuzz-targets"))
        .collect()
}

/// Whole-set fuzz recipes that read no target list, each with the reason the
/// list is not theirs to read.
const FUZZ_RECIPES_THAT_ENUMERATE_NOTHING: &[(&str, &str)] = &[(
    "fuzz-build",
    "`cargo fuzz build` with no target argument compiles every `[[bin]]` the fuzz \
     manifest declares. That enumeration is cargo-fuzz's own and is one step closer \
     to the registry than `cargo fuzz list` is, so reading the list here would be a \
     copy rather than a cure",
)];

/// Fuzz recipes that spell a target's name themselves, each with the reason.
/// Every entry is a LIVE DEFECT rather than an exemption: the name is a
/// hand-maintained copy of one line of `cargo fuzz list`, and the whole point
/// of the derivation is that there are none of those.
const FUZZ_RECIPES_THAT_NAME_A_TARGET: &[(&str, &str)] = &[(
    "fuzz-seed",
    "`sjis_decode` is the one target whose input is bytes rather than text: it hands \
     them to `decode_sjis`, which rejects a UTF-8 seed at its first multi-byte \
     character, so its seeds are transcoded to CP932 and every other target's are \
     not. The encoding a target wants is a per-target fact with nowhere else in this \
     repo to live — and that is the defect, not the workaround: a second \
     byte-oriented `[[bin]]` would be seeded with UTF-8 it cannot read, `just \
     fuzz-seed` would report its seed count as cheerfully as for the rest, and \
     nothing here would say so",
)];

/// The fuzz targets the regression suite replays: the first argument of each
/// `replay_each(…)` call. The definition of the function is not a call, and
/// its body holds strings of its own.
fn replayed_targets(source: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut rest = source;
    while let Some((before, after)) = rest.split_once("replay_each(") {
        rest = after;
        if before.ends_with("fn ") {
            continue;
        }
        out.extend(after.split('"').nth(1).map(str::to_owned));
    }
    out
}

#[test]
fn every_fuzz_target_the_crate_registers_is_one_every_sweep_actually_sweeps() {
    // There were four hand-written listings of the same four names, and
    // DEV-230 was filed believing there were three targets. Three of the four
    // are gone: `fuzz-all-quick`, `fuzz-all-deep` and `fuzz-status` loop over
    // `just _fuzz-targets`, i.e. over `cargo fuzz list`, which reads the very
    // `[[bin]]` tables `cargo fuzz run <name>` resolves against. So the
    // question asked of them changed with them — not "do the copies still
    // agree" but "is either of them a copy". A target name spelled in one of
    // these bodies is the list growing back, and a list that agrees today is
    // exactly what a list about to drift looks like.
    let justfile = read("Justfile");
    let registered = registered_fuzz_targets();
    assert!(
        registered.len() >= 3,
        "the fuzz manifest came out registering {registered:?}; the reader is not finding its \
         `[[bin]]` tables"
    );

    // Over every recipe in the group rather than over the three that used to
    // hold a list. Naming them would have left the check exactly as wide as
    // the defect it was written for and no wider: `fuzz-seed` arrived after
    // those three, whole-set like them, and a fourth hand-written listing in
    // it would have passed a test that knew three names.
    let fuzz_recipes = recipes_in_group(&justfile, "fuzz");
    let whole_set = whole_set_fuzz_recipes(&justfile);
    assert!(
        fuzz_recipes.len() >= 8 && whole_set.len() >= 4,
        "`[group('fuzz')]` came out as {fuzz_recipes:?}, of which {whole_set:?} take no \
         argument; the reader is not finding the attribute or is not finding the headers"
    );

    for recipe in &whole_set {
        let excused = FUZZ_RECIPES_THAT_ENUMERATE_NOTHING
            .iter()
            .any(|&(name, _)| name == recipe);
        assert!(
            excused || runs_recipe(&recipe_body(&justfile, recipe), "_fuzz-targets"),
            "`just {recipe}` takes no argument, so it acts on every registered target, and it \
             never asks `just _fuzz-targets` which those are. Read the registry, or add it to \
             FUZZ_RECIPES_THAT_ENUMERATE_NOTHING with the reason the list is not its to read."
        );
    }

    for recipe in &fuzz_recipes {
        let spelled: Vec<String> = recipe_words(&justfile, recipe)
            .intersection(&registered)
            .cloned()
            .collect();
        let excused = FUZZ_RECIPES_THAT_NAME_A_TARGET
            .iter()
            .any(|&(name, _)| name == recipe);
        assert!(
            spelled.is_empty() || excused,
            "`just {recipe}` names {spelled:?} itself. The list comes from `cargo fuzz list`; \
             writing a target out here gives it a second, hand-maintained definition — the \
             arrangement that let a fourth `[[bin]]` exist while three recipes swept three. \
             If the name is unavoidable, put it in FUZZ_RECIPES_THAT_NAME_A_TARGET, where it \
             is recorded as the defect it is."
        );
    }

    // An excuse for a recipe that no longer exists is an excuse nobody reads,
    // and it is how a table like this comes to bless a name that has since
    // been given to something else.
    for &(name, why) in FUZZ_RECIPES_THAT_ENUMERATE_NOTHING
        .iter()
        .chain(FUZZ_RECIPES_THAT_NAME_A_TARGET)
    {
        assert!(
            fuzz_recipes.contains(name),
            "`{name}` is excused (\"{why}\") and is not a `[group('fuzz')]` recipe any more"
        );
    }

    // The one listing that cannot be derived, and therefore the one still
    // checked by comparison: each target gets its own `#[test]` with its own
    // assertions in `tests/fuzz_regressions.rs`, so the names are written out
    // there and something has to hold them to the manifest.
    let replayed = replayed_targets(&read(
        "crates/aozora-flavored-markdown/tests/fuzz_regressions.rs",
    ));
    assert_eq!(
        replayed,
        registered,
        "`tests/fuzz_regressions.rs` and the fuzz manifest disagree about what the fuzz targets \
         are.\n  registered, not replayed: {:?}\n  replayed, not registered: {:?}",
        registered.difference(&replayed).collect::<Vec<_>>(),
        replayed.difference(&registered).collect::<Vec<_>>()
    );
}

/// Where a triaged crash is pinned once it is promoted out of `fuzz/`.
const REGRESSION_ROOT: &str = "crates/aozora-flavored-markdown/tests/fuzz_regressions";

#[test]
fn every_pinned_regression_sits_under_a_target_the_suite_still_replays() {
    // The same shape one directory over, and the reason the test above is not
    // enough on its own. `replay_each` returns green when it finds no
    // artifacts, and the walk it uses returns nothing for a directory that is
    // not there — so renaming a fuzz target, or moving its folder, takes its
    // pinned crashes out of the suite while every test still passes. Seven
    // artifacts are pinned here today; the count is asserted because a reader
    // that found none would report the same silence as a suite replaying none.
    let registered = registered_fuzz_targets();
    let root = repo_root().join(REGRESSION_ROOT);
    let mut pinned = 0usize;
    let mut orphaned = Vec::new();

    for entry in fs::read_dir(&root)
        .unwrap_or_else(|e| panic!("reading {}: {e}", root.display()))
        .flatten()
    {
        let path = entry.path();
        let Some(target) = path
            .file_name()
            .and_then(|name| name.to_str())
            .filter(|_| path.is_dir())
        else {
            continue;
        };
        // The same filter the suite applies: a companion `.txt` / `.md` beside
        // an artifact is archaeology, not an input, and a dotfile is neither —
        // `.gitkeep` is how a target with nothing promoted yet still owns a
        // directory, which is what makes a MISSING one a failure over there.
        let artifacts = fs::read_dir(&path)
            .into_iter()
            .flatten()
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| {
                let hidden = path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with('.'));
                path.is_file()
                    && !hidden
                    && path
                        .extension()
                        .is_none_or(|ext| ext != "txt" && ext != "md")
            })
            .count();
        if artifacts == 0 {
            continue;
        }
        pinned += artifacts;
        if !registered.contains(target) {
            orphaned.push(format!("{target} ({artifacts} artifact(s))"));
        }
    }

    assert!(
        orphaned.is_empty(),
        "these directories under {REGRESSION_ROOT} hold pinned crashes and name no registered \
         fuzz target: {orphaned:?}\n\
         The suite replays a directory per registered target, so nothing reads these — the \
         regressions they pin are unpinned and no test says so."
    );
    assert!(
        pinned > 0,
        "no pinned regression artifact found under {REGRESSION_ROOT}; either every promoted \
         crash has been deleted, or this reader is looking in the wrong place — and a promoted \
         crash is never deleted."
    );
}

// ---------------------------------------------------------------------------
// what has to exist before a fuzz gate can mean anything
// ---------------------------------------------------------------------------
//
// The section above asks where the names are written. This one asks whether
// the directories those names stand for are there at all, because both of the
// suites that read them answer "not there" and "nothing in it" the same way,
// and one of those two answers is a pass.

/// Where `just fuzz-seed` installs the committed corpus.
const CORPUS_ROOT: &str = "crates/aozora-flavored-markdown/fuzz/corpus";

/// The lockfile the fuzz workspace resolves against. It is this repo's
/// substitute for a `--locked` cargo-fuzz does not offer, so it has to be a
/// file rather than a sentence.
const FUZZ_LOCK: &str = "crates/aozora-flavored-markdown/fuzz/Cargo.lock";

/// What tells a seed from libFuzzer's own output. The two live in one
/// directory — a corpus is both the set you start from and the set the fuzzer
/// grows — and only the first half is committed, so the prefix is what
/// `.gitignore` re-includes and what this file counts.
const SEED_PREFIX: &str = "seed-";

/// The seed source that carries this crate's own dialect. The rest of the
/// corpus is the CommonMark and GFM spec examples, which are exactly the input
/// class comrak already handles: without these seven documents a fuzzer can
/// mutate for an hour without ever emitting a ruby annotation, and the aozora
/// layer is the whole reason this crate exists rather than a comrak dependency.
const SEED_SOURCE: &str = "playground/examples";

/// The files git would carry under `relative`: tracked ones, plus untracked
/// ones that no ignore rule excludes.
///
/// Both halves, because this question has to give the same answer before and
/// after the commit that adds the files — a check that only counted tracked
/// files would fail on the branch that creates them, and one that only looked
/// at the filesystem would pass on a tree where `.gitignore` guarantees the
/// next clone gets nothing. That second state is not hypothetical: it is what
/// `crates/*/fuzz/corpus/` was until this PR, with a populated corpus on the
/// author's disk and an empty one on every runner.
fn git_carried(relative: &str) -> BTreeSet<String> {
    let out = Command::new("git")
        .args([
            "ls-files",
            "--cached",
            "--others",
            "--exclude-standard",
            "--",
            relative,
        ])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running `git ls-files -- {relative}`: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where git is installed and \
                 the work tree is mounted."
            )
        });
    assert!(
        out.status.success(),
        "`git ls-files -- {relative}` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::to_owned)
        .collect()
}

/// The last segment of a `/`-separated path.
fn file_name_of(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// Every `.md` document under `SEED_SOURCE`, as (name, bytes).
fn seed_source_documents() -> Vec<(String, Vec<u8>)> {
    let dir = repo_root().join(SEED_SOURCE);
    let mut out: Vec<(String, Vec<u8>)> = fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
        .map(|path| {
            let bytes =
                fs::read(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
            (label_of(&path), bytes)
        })
        .collect();
    out.sort();
    out
}

#[test]
fn every_registered_fuzz_target_owns_a_regression_directory_a_clone_would_get() {
    // The other direction of `every_pinned_regression_sits_under_a_target_the
    // _suite_still_replays`, and the one that was missing while that test's own
    // comment described the failure exactly: "`replay_each` returns green when
    // it finds no artifacts, and the walk it uses returns nothing for a
    // directory that is not there". It then checked only that every directory
    // present names a registered target — never that every registered target
    // has a directory. Three of the four did not. `render_blocks`,
    // `serialize_round_trip` and `sjis_decode` each had a `#[test]` that read
    // no byte, asserted nothing and reported success, and every gate in this
    // repo was green over it.
    //
    // The suite hard-fails on a missing directory now, so this is the same
    // statement asked one step earlier and one step wider: earlier because it
    // fails in the xtask suite rather than in three separate integration tests,
    // and wider because a directory that exists only on the author's disk is
    // the same nothing to a runner as one that does not exist at all.
    let registered = registered_fuzz_targets();
    assert!(
        registered.len() >= 3,
        "the fuzz manifest came out registering {registered:?}; the reader is not finding its \
         `[[bin]]` tables"
    );

    for target in &registered {
        let relative = format!("{REGRESSION_ROOT}/{target}");
        assert!(
            repo_root().join(&relative).is_dir(),
            "`{target}` is a registered fuzz target and {relative} does not exist. Its promoted \
             crashes have nowhere to be, and until the suite started failing on this the test \
             that replays them passed by replaying none."
        );
        let carried = git_carried(&relative);
        assert!(
            !carried.is_empty(),
            "{relative} exists here and git carries nothing in it, so the next clone has no such \
             directory — which is the state the replay suite fails on. Git stores no empty \
             directory: commit the promoted artifacts, or the `.gitkeep` that stands in for them."
        );
    }
}

#[test]
fn every_registered_fuzz_target_starts_from_a_seed_corpus_a_clone_would_get() {
    // A fuzzer handed no corpus starts from one empty byte string, and the
    // first minutes of every run go on rediscovering that markdown has
    // headings. `crates/*/fuzz/corpus/` was in `.gitignore` wholesale, so that
    // was the state of every CI run and every fresh checkout — while the author
    // who ran `just fuzz-all-deep` locally had a corpus grown over previous
    // runs and saw a completely different search.
    //
    // Nothing could have reported it. The corpus is an input to a workflow that
    // is deliberately not a required check, so its absence does not fail
    // anything; it just quietly makes the sweep worth much less than the
    // 20 minutes it costs.
    let registered = registered_fuzz_targets();
    assert!(
        registered.len() >= 3,
        "the fuzz manifest came out registering {registered:?}; the reader is not finding its \
         `[[bin]]` tables"
    );

    let mut seeded: BTreeMap<String, BTreeSet<Vec<u8>>> = BTreeMap::new();
    for target in &registered {
        let relative = format!("{CORPUS_ROOT}/{target}");
        let all = git_carried(&relative);
        // The control on the reader, and a rule in its own right: a corpus
        // directory is also libFuzzer's scratch space, where every input it
        // finds interesting lands under a SHA-1 name. Tens of thousands of
        // those accumulate on a machine that fuzzes, and `git add -A` on a tree
        // whose ignore rules had lapsed would commit the lot.
        let strays: Vec<&String> = all
            .iter()
            .filter(|path| !file_name_of(path).starts_with(SEED_PREFIX))
            .collect();
        assert!(
            strays.is_empty(),
            "git would carry {} file(s) under {relative} that are not seeds, e.g. {:?}. \
             Everything in a corpus directory except `{SEED_PREFIX}*` is libFuzzer's own output.",
            strays.len(),
            strays.iter().take(3).collect::<Vec<_>>()
        );
        let carried: Vec<String> = all
            .into_iter()
            .filter(|path| file_name_of(path).starts_with(SEED_PREFIX))
            .collect();
        assert!(
            !carried.is_empty(),
            "`{target}` is a registered fuzz target and git carries no `{SEED_PREFIX}*` file \
             under {relative}. Every run of it on a runner therefore starts from an empty \
             corpus. `just fuzz-seed` writes them; `.gitignore` decides whether a clone gets \
             them."
        );
        let bytes = carried
            .iter()
            .map(|path| {
                let full = repo_root().join(path);
                fs::read(&full).unwrap_or_else(|e| panic!("reading {}: {e}", full.display()))
            })
            .collect();
        seeded.insert(target.clone(), bytes);
    }

    // And what is IN them. A corpus of the right size made of the wrong
    // documents is the failure this repo is most exposed to: the spec examples
    // are pure CommonMark and GFM, i.e. precisely the input class comrak is
    // already fuzzed on upstream, and a seed set of nothing else would leave
    // the aozora layer — the reason this crate exists — unreached by every
    // sweep while the counts above all looked healthy.
    let documents = seed_source_documents();
    assert!(
        documents.len() >= 5,
        "{SEED_SOURCE} came out holding {documents:?}; the reader is not finding the documents \
         `just fuzz-seed` copies"
    );
    for (name, body) in &documents {
        assert!(
            seeded.values().any(|seeds| seeds.contains(body)),
            "{name} is a seed source and no registered target's corpus holds it verbatim. These \
             are the documents that carry ruby, bouten, tate-chu-yoko and the paired containers \
             into the search; the spec examples that make up the rest of the corpus carry none \
             of them."
        );
    }
}

#[test]
fn the_lockfile_that_stands_in_for_the_missing_flag_is_one_a_clone_would_get() {
    // cargo-fuzz has no `--locked` and no way to hand one to the `cargo build`
    // it shells out to, so the fuzz workspace's resolution is bound by a
    // committed lockfile instead, and `just fuzz-build` fails when a build
    // rewrote it. That substitute is `git diff --quiet -- fuzz/Cargo.lock` —
    // which says nothing at all about a file git is not tracking. An ignored
    // or simply un-added lockfile makes the gate a silent no-op with every
    // message in it still perfectly accurate, and `fuzz/.gitignore` listed
    // `Cargo.lock` until this PR.
    //
    // What this asks is "would a clone get it", which covers the ignore rule
    // and does not yet cover the index: on the branch that first adds the file,
    // it is carried and still untracked, and `git diff` reports nothing about
    // an untracked path either. That gap closes when the file is committed and
    // stays closed — nothing can make a tracked file untracked by accident —
    // but until then the recipe's check is the weaker of the two.
    let carried = git_carried(FUZZ_LOCK);
    assert!(
        carried.contains(FUZZ_LOCK),
        "git would not carry {FUZZ_LOCK} to the next clone. `just fuzz-build` detects a \
         re-resolution with `git diff` against that file, and `git diff` over an untracked or \
         ignored path reports nothing — so the check passes on every build, including the ones \
         that resolved a different graph than the library ships."
    );
}

#[test]
fn a_gate_compiles_every_fuzz_target_the_crate_registers() {
    // The fuzz crate declares its own `[workspace]` — correctly, since
    // libfuzzer-sys is nightly-only and must not join a stable `--workspace`
    // build — and the cost of that was total: `cargo check --workspace`,
    // `cargo clippy --workspace` and `cargo build --workspace` have never
    // compiled one line of it. Four harnesses calling this crate's public API
    // by hand sat outside every gate in the repo, so a rename in `src/` broke
    // them silently (DEV-270, DEV-291) and stayed broken until somebody fuzzed
    // by hand.
    //
    // The section above this one already read every `cargo fuzz` invocation in
    // the file — and asked each of them only which triple it names. Every one
    // of them lived in a recipe nothing runs, which is the question it never
    // put: a build flag is worth nothing on a build no gate performs.
    let justfile = read("Justfile");
    let manifest = recipes_in_group(&justfile, "gate");
    let registered = registered_fuzz_targets();
    let builds = fuzz_builds(&justfile);
    assert!(
        builds.len() >= 5 && registered.len() >= 3,
        "{} fuzz build(s) and {registered:?} registered; a reader is not finding one of them",
        builds.len()
    );

    let gated: Vec<&FuzzBuild> = builds
        .iter()
        .filter(|build| manifest.contains(&build.recipe))
        .collect();
    assert!(
        !gated.is_empty(),
        "no `[group('gate')]` recipe compiles the fuzz crate: cargo-fuzz is invoked only by {:?}, \
         and none of those is a gate. The crate is then compiled by nothing a PR runs, which is \
         how four harnesses came to be broken by a rename with every gate green.",
        builds
            .iter()
            .map(|build| &build.recipe)
            .collect::<BTreeSet<_>>()
    );

    for build in gated {
        let tokens = shell_tokens(&build.line);
        let at = fuzz_build_at(&tokens).unwrap_or_else(|| {
            panic!(
                "`just {}`: {} is no longer a fuzz build",
                build.recipe,
                build.line.trim()
            )
        });
        assert_eq!(
            tokens[at], "build",
            "`just {}` gates on `cargo fuzz {}`, which searches rather than compiles. What a PR \
             owes is that the harnesses still match the API they call, and that is a compile; a \
             search belongs in fuzz.yml, where a finding is a bug report rather than a blocked \
             merge.",
            build.recipe, tokens[at]
        );
        let named: Vec<&String> = tokens
            .iter()
            .filter(|token| registered.contains(token.as_str()))
            .collect();
        assert!(
            named.is_empty(),
            "`just {}` gates on a build of {named:?} alone. `cargo fuzz build` with no target \
             argument compiles every `[[bin]]` in the manifest; naming one leaves the others \
             exactly where they were — compiled by nothing.",
            build.recipe
        );
    }
}

/// The workflow that runs the fuzz targets, as opposed to compiling them.
const FUZZ_WORKFLOW: &str = ".github/workflows/fuzz.yml";

/// Where libFuzzer writes an input that broke something, relative to the fuzz
/// crate. Spelled once here and asserted to be what both the recipes and the
/// workflow say.
const FUZZ_ARTIFACT_DIR: &str = "fuzz/artifacts";

/// The workflow step holding line `at`: from the `- ` that opens it to the one
/// that opens the next. A `with:` value belongs to the step above it, and
/// nothing in YAML's indentation says so to a reader that took a fixed window.
fn step_around<'a>(lines: &[&'a str], at: usize) -> Vec<&'a str> {
    let opens = |line: &str| strip_comment(line).trim_start().starts_with("- ");
    let from = lines[..=at]
        .iter()
        .rposition(|line| opens(line))
        .unwrap_or(at);
    let to = lines[at + 1..]
        .iter()
        .position(|line| opens(line))
        .map_or(lines.len(), |offset| at + 1 + offset);
    lines[from..to].to_vec()
}

#[test]
fn the_fuzz_workflow_runs_a_sweep_that_exists_on_each_of_its_events() {
    // `fuzz-all-deep`'s comment has called it "the gate before tagging a
    // release" since it was written, and until DEV-230 gave the recipes a
    // triple they could build for, it had never once run. Nothing scheduled
    // it; a release pre-flight that is only ever invoked by a human on release
    // day is discovered broken on release day.
    //
    // The workflow that fixes that is deliberately NOT a required check — a
    // fuzzer is a search, and a finding is a bug report rather than a reason to
    // block an unrelated PR. Which is exactly why it needs a test: nothing goes
    // red when this file stops working. Rename a sweep, drop the `schedule:`
    // trigger, delete the upload step, and the only symptom is a workflow that
    // keeps passing while searching nothing or keeping nothing.
    let justfile = read("Justfile");
    let workflow = read(FUZZ_WORKFLOW);
    let sweeps = derived_sweeps(&justfile);
    assert!(
        sweeps.len() >= 3,
        "the derived sweeps came out as {sweeps:?}; the reader is not finding the `[group('fuzz')]` \
         recipes that loop over `_fuzz-targets`"
    );

    // What it dispatches has to BE one of those recipes. The sweep arrives
    // through an env var rather than a `run:` argument (zizmor's
    // template-injection rule), so `recipes_invoked` cannot see it and the name
    // is checked against the Justfile here instead of by the first runner to
    // hit `error: Justfile does not contain recipe`.
    let dispatched: BTreeSet<String> = workflow_vocabulary(&workflow)
        .intersection(&sweeps)
        .cloned()
        .collect();
    assert!(
        dispatched.len() >= 2,
        "{FUZZ_WORKFLOW} names {dispatched:?} of the sweeps {sweeps:?}. It runs one per event — a \
         short one per pull request, the release pre-flight on the cron — and a name here that is \
         not a recipe there fails on the runner, in a workflow whose failures block nothing."
    );

    // The deep sweep is the one that runs the release pre-flight, and it is the
    // one the schedule exists for. Which recipe that is, is read off the
    // Justfile rather than spelled here.
    let deep = sweeps
        .iter()
        .find(|sweep| runs_recipe(&recipe_body(&justfile, sweep), "fuzz-deep"))
        .unwrap_or_else(|| {
            panic!(
                "no `[group('fuzz')]` sweep runs `just fuzz-deep` any more; the release \
                    pre-flight has no sweep to be"
            )
        });
    assert!(
        jobs_block(&workflow).iter().any(|line| {
            strip_comment(line).contains("schedule") && strip_comment(line).contains(deep.as_str())
        }),
        "{FUZZ_WORKFLOW} does not pick `{deep}` on `schedule`. The cron is what keeps the release \
         pre-flight warm; a schedule that runs the 60-second sweep instead proves only that the \
         harnesses start."
    );
    for trigger in ["pull_request:", "schedule:"] {
        assert!(
            workflow
                .lines()
                .any(|line| strip_comment(line).trim() == trigger),
            "{FUZZ_WORKFLOW} has no `{trigger}` trigger. Both halves are load-bearing and they are \
             different halves: the pull_request run catches an invariant a change broke on the \
             first mutation of a seed, the cron is the only thing that ever runs the deep sweep."
        );
    }
    assert!(
        workflow
            .lines()
            .any(|line| strip_comment(line).trim().starts_with("- cron:")),
        "{FUZZ_WORKFLOW} declares a `schedule:` with no `cron:` under it"
    );
}

#[test]
fn the_fuzz_workflow_keeps_the_input_a_failing_sweep_found() {
    // The other half of a workflow that blocks nothing: a finding has to
    // survive the runner. libFuzzer writes the input that broke something into
    // `fuzz/artifacts/` and the machine is then destroyed, so without the
    // upload all that reaches a human is a red square on a workflow nobody is
    // required to read — and the crash has to be re-found rather than replayed.
    let justfile = read("Justfile");
    let workflow = read(FUZZ_WORKFLOW);
    let jobs = jobs_block(&workflow);
    assert!(
        jobs.len() >= 10,
        "{FUZZ_WORKFLOW} came out with {} line(s) under `jobs:`; the reader is not finding the \
         mapping",
        jobs.len()
    );
    let uploads = jobs
        .iter()
        .position(|line| strip_comment(line).contains("upload-artifact"))
        .unwrap_or_else(|| {
            panic!(
                "{FUZZ_WORKFLOW} uploads nothing: a crashing input found on a runner is lost \
                 with the runner, and all that is left of it is that something failed"
            )
        });
    let step = step_around(&jobs, uploads);
    assert!(
        step.iter()
            .any(|line| strip_comment(line).contains("failure()")),
        "{FUZZ_WORKFLOW}'s upload step is not conditioned on `failure()`, so it either runs on \
         every green sweep or does not run when there is something to keep:\n{}",
        step.join("\n")
    );
    assert!(
        step.iter()
            .any(|line| strip_comment(line).contains(FUZZ_ARTIFACT_DIR)),
        "{FUZZ_WORKFLOW}'s upload step does not name `{FUZZ_ARTIFACT_DIR}`, which is where \
         libFuzzer writes what it found:\n{}",
        step.join("\n")
    );
    assert!(
        justfile.contains(FUZZ_ARTIFACT_DIR),
        "the Justfile no longer names `{FUZZ_ARTIFACT_DIR}`, so the directory the workflow \
         collects and the directory `just fuzz-triage` replays have drifted apart"
    );
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn a_triple_only_libfuzzer_can_see_is_not_the_triple_cargo_fuzz_builds_for() {
    // Every fuzz recipe here ends in `-- -max_total_time=…`, so the passthrough
    // is the one place a `--target` is easiest to put and the one place it does
    // nothing: libFuzzer takes `-flag=value` and would reject it outright.
    let passed_through = fuzz_builds(concat!(
        "fuzz-x TARGET:\n",
        "    cargo +nightly fuzz run {{TARGET}} -- --target x86_64-unknown-linux-gnu\n",
    ));
    assert_eq!(passed_through.len(), 1);
    assert!(
        passed_through[0].triple.is_none(),
        "a `--target` handed to libFuzzer counted as the triple cargo-fuzz builds for"
    );

    // `--target-dir` is a different flag, and a prefix match would read its
    // value — a path — as a target triple.
    let target_dir = fuzz_builds(concat!(
        "fuzz-y:\n",
        "    cargo +nightly fuzz run --target-dir /cargo/target t\n",
    ));
    assert_eq!(target_dir.len(), 1);
    assert!(
        target_dir[0].triple.is_none(),
        "`--target-dir` was read as `--target`"
    );

    // Both spellings clap accepts, and the `{{VAR}}` the recipes actually use.
    for line in [
        "    cargo +nightly fuzz run --target x86_64-unknown-linux-gnu t -- -max_total_time=60\n",
        "    cargo +nightly fuzz run --target={{_T}} t\n",
        "    {{_fuzz}} bash -c 'cd crates/x && cargo +nightly fuzz build --target {{_T}} t'\n",
    ] {
        let justfile = format!("_T := \"x86_64-unknown-linux-gnu\"\nfuzz-z:\n{line}");
        let builds = fuzz_builds(&justfile);
        assert_eq!(builds.len(), 1, "{line}");
        assert_eq!(
            builds[0].triple.as_deref(),
            Some("x86_64-unknown-linux-gnu"),
            "the triple went unread in: {line}"
        );
        assert_eq!(builds[0].recipe, "fuzz-z", "{line}");
    }

    // A cargo-fuzz sub-command that compiles nothing has no triple to name,
    // and prose about the flag is not the flag.
    let not_a_build = fuzz_builds(concat!(
        "# cargo fuzz run --target x86_64-unknown-linux-gnu is what the recipes do\n",
        "fuzz-l:\n",
        "    cargo +nightly fuzz list\n",
        "    # cargo +nightly fuzz run t\n",
    ));
    assert!(
        not_a_build.is_empty(),
        "`cargo fuzz list`, or a commented-out run, counted as a build: {:?}",
        not_a_build
            .iter()
            .map(|build| &build.line)
            .collect::<Vec<_>>()
    );
}

#[test]
fn a_sweep_reads_its_targets_and_a_definition_is_not_a_call() {
    // The two shapes the derivation check has to tell apart: a loop over the
    // registry, and the hand-written list it replaced. Prose about the old
    // shape has to read as the new one, or the check fails on the comment that
    // explains it — which is how a gate gets its explanation deleted.
    let derived = concat!(
        "fuzz-all-quick:\n",
        "    #!/usr/bin/env bash\n",
        "    # was: just fuzz-quick parse_render\n",
        "    for target in $(just _fuzz-targets); do\n",
        "        just fuzz-quick \"$target\"\n",
        "    done\n",
    );
    let read_here = recipe_words(derived, "fuzz-all-quick");
    assert!(
        read_here.contains("target"),
        "a recipe body came out as {read_here:?}; the reader is not finding its lines"
    );
    assert!(
        !read_here.contains("parse_render"),
        "a target named in a COMMENT counted as the recipe naming it"
    );
    assert!(
        runs_recipe(&recipe_body(derived, "fuzz-all-quick"), "_fuzz-targets"),
        "the call that fetches the list went unread inside `$( … )`"
    );

    let hand_written = concat!(
        "fuzz-all-quick:\n",
        "    just fuzz-quick parse_render\n",
        "    just fuzz-quick sjis_decode\n",
    );
    assert!(
        recipe_words(hand_written, "fuzz-all-quick").contains("parse_render"),
        "a target spelled in a recipe body went unread, so a hand-written list would pass"
    );

    let suite = concat!(
        "fn replay_each(target: &str, assert_one: impl Fn(&str)) {\n",
        "    panic!(\"regression artifact {label} still crashes\");\n",
        "}\n",
        "#[test]\n",
        "fn parse_render_regressions_replay_cleanly() {\n",
        "    replay_each(\n        \"parse_render\",\n        |text| drop(text),\n    );\n",
        "}\n",
    );
    assert_eq!(
        replayed_targets(suite),
        BTreeSet::from(["parse_render".to_owned()]),
        "the reader took a string out of `replay_each`'s own body, or missed the call that \
         spreads its argument onto the next line"
    );
}

// ---------------------------------------------------------------------------
// a scan whose answer changes without a commit
// ---------------------------------------------------------------------------
// The fourth way a check can exist and check nothing, and the one the file
// above cannot see: a gate that is declared, is run, is strict, and is asked
// only at the wrong moments. Every assertion up to here holds a gate to the
// tree it is handed. `just audit` and `just deny` are not functions of the
// tree — they are functions of a database somewhere else, which moves while
// nobody is pushing. Running them on `pull_request` answers "did this diff add
// a finding"; nothing was answering "did one appear since the last diff", and
// SECURITY.md stated that gap as policy ("there is no cron workflow") where no
// reader could act on it.
//
// The other half is what a scheduled run does when it finds something. A cron
// that blocks no merge and reports nowhere is a control only for whoever
// remembers to go and look, so the last section here reads what each schedule
// hands a human.

/// The events a workflow declares under `on:`.
fn triggers(workflow: &str) -> BTreeSet<String> {
    top_level_block(workflow, "on")
        .into_iter()
        .filter_map(job_key)
        .map(str::to_owned)
        .collect()
}

/// Does this workflow run on a clock? Both halves are needed: `schedule:` with
/// no `cron:` under it is a trigger that never fires, and GitHub says nothing
/// about it.
fn runs_on_a_cron(workflow: &str) -> bool {
    triggers(workflow).contains("schedule")
        && top_level_block(workflow, "on")
            .iter()
            .any(|line| strip_comment(line).trim().starts_with("- cron:"))
}

/// Every workflow, read: `(repo-relative label, contents)`.
fn read_workflows() -> Vec<(String, String)> {
    workflow_files()
        .into_iter()
        .map(|path| {
            let label = label_of(&path);
            let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
            (label, text)
        })
        .collect()
}

/// Gates whose verdict is a function of the world outside this repository, and
/// what moves it. Listed by hand because the property is not written anywhere
/// a reader could derive it from — but the names are checked against the
/// manifest below, so an entry cannot quietly stop naming a gate.
const CLOCK_DEPENDENT_GATES: &[(&str, &str)] = &[
    (
        "audit",
        "the RustSec advisory database, which files advisories against lockfiles nobody has touched",
    ),
    (
        "deny",
        "the same database, plus the yanked flag crates.io sets on a version long after it resolved",
    ),
];

#[test]
fn every_gate_whose_answer_changes_without_a_commit_runs_without_one() {
    let justfile = read("Justfile");
    let gates = recipes_in_group(&justfile, "gate");
    let workflows = read_workflows();
    let mut unscheduled = Vec::new();

    for &(gate, moved_by) in CLOCK_DEPENDENT_GATES {
        assert!(
            gates.contains(gate),
            "`{gate}` is not a `[group('gate')]` recipe any more. It was this repo's watch over \
             {moved_by}; a rename that leaves this list behind is how the schedule below stops \
             covering anything while staying green."
        );
        let on_a_cron = workflows
            .iter()
            .any(|(_, text)| runs_on_a_cron(text) && recipes_invoked(text).contains(gate));
        if !on_a_cron {
            unscheduled.push(format!(
                "  `just {gate}` watches {moved_by}, and no workflow runs it on a `cron:`. Its \
                 pull-request run answers whether THIS diff introduced a finding. Nothing answers \
                 whether one appeared against a lockfile nobody edited, which is how they \
                 normally appear."
            ));
        }
    }

    assert!(
        unscheduled.is_empty(),
        "gates that are only ever asked about a diff:\n{}",
        unscheduled.join("\n")
    );
}

#[test]
fn the_two_advisory_scanners_fail_on_the_same_findings() {
    // The pair is only a pair if both halves have the same idea of a finding.
    // cargo-deny's `yanked = "deny"` is in `deny.toml`; cargo-audit's default
    // exit status counts vulnerabilities ONLY — `unmaintained`, `unsound`,
    // `notice` and a yanked crate are printed and exit 0 — so without
    // `--deny warnings` the two disagree, and the half that runs on the
    // lockfile is the lax one. That was the live state: the flag existed once,
    // on the vendored-comrak `audit-comrak` recipe, and left with it.
    let deny_toml = read("deny.toml");
    let yanked =
        manifest_value(&deny_toml, "advisories", "yanked").map(|value| value.trim_matches('"'));
    assert_eq!(
        yanked,
        Some("deny"),
        "deny.toml no longer denies yanked crates. That is a decision about what counts as a \
         finding, and it has to be taken for both scanners at once — this test is where they are \
         held together."
    );

    let justfile = read("Justfile");
    let scans: Vec<(String, String)> = expanded_recipe_lines(&justfile)
        .into_iter()
        .filter(|(_, line)| {
            tool_commands(line)
                .iter()
                .any(|(tool, sub)| tool == "cargo" && sub == "audit")
        })
        .collect();
    assert!(
        !scans.is_empty(),
        "no recipe in the Justfile runs `cargo audit` any more, so the RustSec half of the pair \
         is gone and `deny.toml` is carrying the advisory policy alone"
    );

    let lax: Vec<String> = scans
        .iter()
        .filter(|(_, line)| !denies_warnings(line))
        .map(|(recipe, line)| format!("  {recipe}: {}", line.trim()))
        .collect();
    assert!(
        lax.is_empty(),
        "`cargo audit` without `--deny warnings`, while deny.toml denies yanked crates:\n{}\n\
         cargo-audit exits 0 on everything RustSec files as a warning, so this scan can only fail \
         on a live vulnerability — it prints the rest into a log nobody reads.",
        lax.join("\n")
    );
}

/// The reporter that turns an advisory into something with an assignee.
/// `rustsec/audit-check` opens one issue per newly disclosed advisory when
/// `github.event_name` is `schedule`, and writes a check run on every other
/// event. So the schedule is not decoration around it — drop the trigger and
/// the workflow still runs, still goes red, and files nothing.
const ADVISORY_REPORTER: &str = "rustsec/audit-check";

#[test]
fn a_new_advisory_arrives_as_an_issue_and_not_only_as_a_red_square() {
    let workflows = read_workflows();
    let scheduled: Vec<&(String, String)> = workflows
        .iter()
        .filter(|(_, text)| {
            runs_on_a_cron(text)
                && CLOCK_DEPENDENT_GATES
                    .iter()
                    .any(|&(gate, _)| recipes_invoked(text).contains(gate))
        })
        .collect();
    assert!(
        !scheduled.is_empty(),
        "no workflow runs an advisory scan on a cron, so there is nothing here to report from"
    );

    for (label, text) in scheduled {
        let job = job_keys(text)
            .into_iter()
            .find(|job| {
                job_lines(text, job).is_some_and(|lines| {
                    lines
                        .iter()
                        .any(|line| strip_comment(line).contains(ADVISORY_REPORTER))
                })
            })
            .unwrap_or_else(|| {
                panic!(
                    "{label} runs an advisory scan nightly and no job in it uses \
                     `{ADVISORY_REPORTER}`. A cron whose only output is a red square on a \
                     workflow that blocks no merge notifies nobody; the issue is the control."
                )
            });

        let lines = job_lines(text, &job).unwrap_or_default();
        assert!(
            lines
                .iter()
                .any(|line| strip_comment(line).trim() == "issues: write"),
            "{label}'s `{job}` job runs {ADVISORY_REPORTER} without `issues: write`. The action \
             falls back to a check run it cannot fail on, so the scheduled run reports into the \
             same place nobody is looking."
        );

        assert!(
            job_needs(text, &job).is_empty(),
            "{label}'s `{job}` job waits on {:?}. A reporter downstream of the scan is skipped \
             exactly when the scan fails, which is the only run whose finding anyone needed.",
            job_needs(text, &job)
        );
    }
}

/// Files that describe this repository's own CI to a reader. History is out:
/// `CHANGELOG.md` and the ADRs record what was true when they were written,
/// and editing them to match today is how a record stops being one.
const CI_PROSE_ROOTS: &[&str] = &["", "docs"];
const CI_PROSE_HISTORY: &[&str] = &["CHANGELOG.md", "docs/adr"];

/// Words that deny, and the scheduled-run nouns they must not be attached to.
/// The sentence this exists for is SECURITY.md's "Both ride every pull
/// request; there is no cron workflow." — a security policy asserting the
/// absence of a control, which nothing evaluated, and which stayed on the page
/// for as long as it took somebody to notice.
const NEGATIONS: &[&str] = &["no", "not", "never", "without", "neither"];
const SCHEDULE_NOUNS: &[&str] = &[
    "cron",
    "crons",
    "nightly",
    "nightlies",
    "scheduled",
    "schedule",
];

/// How far after a negation a noun still belongs to it: "no cron workflow",
/// "does not run on a schedule", "never runs a nightly". Four words, because
/// three stops one short of the commonest phrasing of the claim.
const NEGATION_REACH: usize = 4;

/// The lowercase words of a line, hyphens kept so `pull-request` stays one.
fn prose_words(line: &str) -> Vec<String> {
    line.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-'))
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Does this line deny that a scheduled run exists?
fn denies_a_schedule(line: &str) -> bool {
    let words = prose_words(line);
    words.iter().enumerate().any(|(at, word)| {
        NEGATIONS.contains(&word.as_str())
            && words
                .iter()
                .skip(at + 1)
                .take(NEGATION_REACH)
                .any(|later| SCHEDULE_NOUNS.contains(&later.as_str()))
    })
}

/// Every `.md` that describes this repo rather than recording its past.
fn ci_prose_files() -> Vec<(String, String)> {
    let root = repo_root();
    let mut out = Vec::new();
    for directory in CI_PROSE_ROOTS {
        let path = root.join(directory);
        let mut entries: Vec<PathBuf> = fs::read_dir(&path)
            .unwrap_or_else(|e| panic!("reading {}: {e}", path.display()))
            .flatten()
            .map(|entry| entry.path())
            .filter(|path| path.extension().is_some_and(|ext| ext == "md"))
            .collect();
        entries.sort();
        for file in entries {
            let label = label_of(&file);
            if CI_PROSE_HISTORY
                .iter()
                .any(|history| label.starts_with(history))
            {
                continue;
            }
            let text = fs::read_to_string(&file).unwrap_or_else(|e| panic!("reading {label}: {e}"));
            out.push((label, text));
        }
    }
    out
}

#[test]
fn no_document_denies_a_scheduled_run_this_repo_makes() {
    let documents = ci_prose_files();
    assert!(
        documents.iter().any(|(label, _)| label == "SECURITY.md"),
        "the reader is not finding the documents; it found {:?}",
        documents.iter().map(|(label, _)| label).collect::<Vec<_>>()
    );

    let mut denials = Vec::new();
    for (label, text) in &documents {
        for (index, line) in text.lines().enumerate() {
            if denies_a_schedule(line) {
                denials.push(format!("  {label}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        denials.is_empty(),
        "prose that denies a scheduled run:\n{}\n\
         Every clock-driven workflow in this repo is one somebody reading these files is being \
         told does not exist. A sentence about CI that nothing evaluates is a claim with the \
         authority of policy and the lifespan of a guess.",
        denials.join("\n")
    );

    // The other direction, because deleting the sentence is not the same as
    // describing what replaced it: the document that discusses the advisory
    // gates has to name the workflow that schedules them, or the reader is
    // simply told less than before.
    let security = read("SECURITY.md");
    let scheduling: Vec<String> = read_workflows()
        .into_iter()
        .filter(|(_, text)| {
            runs_on_a_cron(text)
                && CLOCK_DEPENDENT_GATES
                    .iter()
                    .any(|&(gate, _)| recipes_invoked(text).contains(gate))
        })
        .map(|(label, _)| label)
        .collect();
    assert!(
        scheduling
            .iter()
            .any(|label| security.contains(label.as_str())),
        "SECURITY.md discusses `just audit` and `just deny` and names none of {scheduling:?}, the \
         workflows that actually re-run them. It described the gap when the gap was real; it has \
         to describe the control now that the control is."
    );
}

/// Where the per-package licence waivers live, and where the watch over them
/// does.
const LICENCE_REVIEW: &str = ".github/workflows/dependency-review.yml";
const LICENCE_WATCH: &str = ".github/workflows/audit.yml";

/// The cargo packages `dependency-review.yml` waives the licence check for.
/// The waiver is per PACKAGE, not per licence: it says "whatever this package
/// declares is fine", for as long as the entry is there.
fn exempted_packages(review: &str) -> BTreeSet<String> {
    Regex::new(r"pkg:cargo/([A-Za-z0-9_.-]+)")
        .expect("the purl pattern is a literal")
        .captures_iter(review)
        .map(|caught| caught[1].to_owned())
        .collect()
}

/// What the watch holds each of them to: the bash table `[name]='licence'`.
fn watched_licences(watch: &str) -> BTreeMap<String, String> {
    Regex::new(r"(?m)^\s*\[([A-Za-z0-9_.-]+)\]='([^']*)'\s*$")
        .expect("the table pattern is a literal")
        .captures_iter(watch)
        .map(|caught| (caught[1].to_owned(), caught[2].to_owned()))
        .collect()
}

#[test]
fn every_licence_exemption_is_pinned_to_the_licence_it_was_written_for() {
    let review = read(LICENCE_REVIEW);
    assert!(
        repo_root().join(LICENCE_WATCH).is_file(),
        "{LICENCE_REVIEW} waives the licence check per package and {LICENCE_WATCH} is gone, so \
         nothing holds a waived package to what it declared when somebody decided to waive it"
    );
    let watch = read(LICENCE_WATCH);
    let exempted = exempted_packages(&review);
    let watched = watched_licences(&watch);

    assert!(
        exempted.len() >= 3,
        "{LICENCE_REVIEW} parsed as {exempted:?}; the reader is not finding the \
         `allow-dependencies-licenses` purls, so everything below passes by reading nothing"
    );

    let watched_names: BTreeSet<String> = watched.keys().cloned().collect();
    assert_eq!(
        watched_names, exempted,
        "the cargo packages {LICENCE_REVIEW} exempts and the ones {LICENCE_WATCH} watches have \
         come apart. An exemption with nothing watching it waives whatever the package declares \
         next — a relicence would pass unremarked and the `Drop this entry once …` sentence \
         beside it would never come due. A watch with no exemption left is a check against \
         nothing. Neither drifts on its own; both drift when one file is edited."
    );

    // The expected string is a claim about the outside world, and only the
    // nightly run can ask crates.io whether it still holds. What is decidable
    // here is that the two files make the SAME claim: the prose in the review
    // says what each package declares and why that is tolerable, and the watch
    // is the executable copy of it.
    let mismatched: Vec<String> = watched
        .iter()
        .filter(|(_, licence)| !review.contains(licence.as_str()))
        .map(|(name, licence)| format!("  {name}: watched as `{licence}`"))
        .collect();
    assert!(
        mismatched.is_empty(),
        "licences {LICENCE_WATCH} watches for that {LICENCE_REVIEW} never says the package \
         declares:\n{}\n\
         The reason an exemption exists is written as prose next to it; if the watch is holding \
         the package to a different string, one of the two was edited and the other was not, and \
         the nightly failure that follows will point at the wrong file.",
        mismatched.join("\n")
    );
}

/// Scheduled workflows whose only output is a red square, each with why it is
/// still like that. Every entry is a LIVE DEFECT, not an exemption: a cron
/// that blocks no merge and reports nowhere is a control only for whoever
/// remembers to go and look at it, and nobody does.
const CRON_WITHOUT_A_CHANNEL: &[(&str, &str)] = &[(
    ".github/workflows/release-pins.yml",
    "predates this rule. Its `freshness` job compares dist-workspace.toml's hand-maintained \
     action pins against upstream and exits 1 into a workflow that blocks no merge, uploads \
     nothing and files nothing — the exact shape audit.yml's `report` job exists to avoid. It \
     needs the same treatment and that is its own change",
)];

/// How a workflow gets a finding off the runner and in front of somebody.
fn reporting_channel(workflow: &str) -> Option<&'static str> {
    let grants = |permission: &str| {
        let wanted = format!("{permission}: write");
        workflow
            .lines()
            .any(|line| strip_comment(line).trim() == wanted)
    };
    if grants("issues") {
        return Some("opens an issue");
    }
    if grants("security-events") {
        return Some("raises a code-scanning alert");
    }
    if jobs_block(workflow)
        .iter()
        .any(|line| strip_comment(line).contains("upload-artifact"))
    {
        return Some("uploads the evidence");
    }
    None
}

#[test]
fn a_scheduled_run_that_blocks_nothing_says_where_its_failure_goes() {
    // The generalisation of the fuzz workflow's upload step and of audit.yml's
    // `report` job. Neither is a required check — a fuzzer finding is a bug
    // report, an advisory is somebody else's publication — so nothing about
    // either failing stops a merge or lands in a pull request. Whatever
    // reaches a human has to be arranged by the workflow itself.
    //
    // The rule is per WORKFLOW, which is where it stops short: audit.yml
    // passes on the strength of its `report` job, and that job reports
    // cargo-audit findings alone. Its `licences` job — the watch over the
    // dependency-review waivers — fails into exactly the red square this test
    // is about, one level below where the test can see it.
    let mut silent = Vec::new();
    let workflows = read_workflows();
    let scheduled: Vec<&(String, String)> = workflows
        .iter()
        .filter(|(_, text)| runs_on_a_cron(text))
        .collect();
    assert!(
        scheduled.len() >= 3,
        "only {} workflow(s) read as running on a cron; the trigger reader has stopped finding \
         them and everything below passes vacuously",
        scheduled.len()
    );

    for (label, text) in &scheduled {
        if CRON_WITHOUT_A_CHANNEL
            .iter()
            .any(|&(known, _)| known == label)
        {
            continue;
        }
        if reporting_channel(text).is_none() {
            silent.push(format!(
                "  {label}: runs on a cron, blocks no merge, and neither files an issue, raises a \
                 code-scanning alert nor uploads an artifact. A red square on a workflow nobody \
                 is required to read is not a control."
            ));
        }
    }
    assert!(
        silent.is_empty(),
        "scheduled runs whose findings reach nobody:\n{}",
        silent.join("\n")
    );

    // A recorded defect that has been fixed is worse than an unrecorded one:
    // it is a live entry vouching for a state that no longer exists.
    for &(label, why) in CRON_WITHOUT_A_CHANNEL {
        let text = read(label);
        assert!(
            runs_on_a_cron(&text),
            "{label} no longer runs on a cron, so it is not in scope for this rule any more — \
             drop its CRON_WITHOUT_A_CHANNEL entry ({why})"
        );
        assert!(
            reporting_channel(&text).is_none(),
            "{label} now {} — the defect its CRON_WITHOUT_A_CHANNEL entry describes is fixed, so \
             delete the entry rather than leaving it to vouch for a state that is gone",
            reporting_channel(&text).unwrap_or_default()
        );
    }
}

#[test]
fn a_trigger_is_read_off_the_on_block_and_a_job_named_alike_is_not() {
    // The mistake a line-at-a-time reader makes here: `schedule` appearing
    // anywhere in a workflow — a job called `schedule`, a `github.event_name
    // == 'schedule'` expression, a comment about the cron — counting as the
    // trigger. fuzz.yml has all three.
    let workflow = concat!(
        "name: probe\n",
        "\n",
        "on:\n",
        "  push:\n",
        "    branches: [main]\n",
        "  schedule:\n",
        "    # off the hour\n",
        "    - cron: \"40 15 * * *\"\n",
        "  workflow_dispatch:\n",
        "\n",
        "jobs:\n",
        "  schedule:\n",
        "    steps:\n",
        "      - run: echo \"${{ github.event_name == 'schedule' }}\"\n",
    );
    assert_eq!(
        triggers(workflow),
        BTreeSet::from([
            "push".to_owned(),
            "schedule".to_owned(),
            "workflow_dispatch".to_owned()
        ]),
        "the `on:` reader took a nested key or missed one"
    );
    assert!(
        runs_on_a_cron(workflow),
        "a `schedule:` with a `cron:` under it was not read as one"
    );

    let no_trigger = workflow.replace(
        "  schedule:\n    # off the hour\n    - cron: \"40 15 * * *\"\n",
        "",
    );
    assert!(
        !runs_on_a_cron(&no_trigger),
        "a job called `schedule` and an expression mentioning it were read as the trigger"
    );

    let empty_schedule = concat!(
        "on:\n",
        "  schedule:\n",
        "  workflow_dispatch:\n",
        "jobs:\n",
        "  sweep:\n",
        "    steps: []\n",
    );
    assert!(
        !runs_on_a_cron(empty_schedule),
        "a `schedule:` with no `cron:` under it fires never, and was read as a clock"
    );
}

#[test]
fn a_denial_of_a_schedule_is_read_and_a_description_of_one_is_not() {
    for denial in [
        "request; there is no cron workflow.",
        "These do not run on a schedule.",
        "The advisory scan never runs nightly.",
    ] {
        assert!(denies_a_schedule(denial), "a denial went unread: {denial}");
    }
    for description in [
        "The schedule is not redundant with the pull request: an advisory is",
        "That nightly run also files a GitHub issue per newly disclosed advisory,",
        "published against a `Cargo.lock` nobody has touched, which is the one case",
        "Not a PR gate: it depends on upstream release timing, so it must never redden",
    ] {
        assert!(
            !denies_a_schedule(description),
            "prose describing a schedule was read as denying one: {description}"
        );
    }
}
