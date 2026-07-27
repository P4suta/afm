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
//! about a recipe's NAME. The last section reads what a gate hands its tool,
//! holds the rustdoc build the repo publishes to the shape docs.rs will
//! build, and runs rustdoc against a probe so that "these flags deny" is
//! measured rather than spelled.

#![forbid(unsafe_code)]

use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU32, Ordering};

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

/// The lines under the workflow's `jobs:` mapping. `on:`, `env:` and
/// `permissions:` hold keys indented exactly like a job and are not jobs.
fn jobs_block(workflow: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let mut in_jobs = false;
    for line in workflow.lines() {
        if line.starts_with([' ', '\t']) || line.trim().is_empty() {
            if in_jobs {
                out.push(line);
            }
            continue;
        }
        // A column-zero comment interrupts nothing; any other column-zero line
        // is the next top-level key.
        if !line.starts_with('#') {
            in_jobs = line.trim_end() == "jobs:";
        }
    }
    out
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
fn plain_variables(justfile: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for line in justfile.lines() {
        if line.starts_with([' ', '\t', '#', '[']) {
            continue;
        }
        let Some((name, value)) = line.split_once(":=") else {
            continue;
        };
        let Some(literal) = value
            .trim()
            .strip_prefix('"')
            .and_then(|rest| rest.strip_suffix('"'))
        else {
            continue;
        };
        out.insert(name.trim().to_owned(), literal.to_owned());
    }
    out
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

/// Every documentation build in the `Justfile`, attributed to its recipe.
fn doc_builds(justfile: &str) -> Vec<DocBuild> {
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
        let expanded = expand(strip_comment(line), &variables);
        if !builds_rustdoc(&expanded) {
            continue;
        }
        let flags = rustdocflags_on(&expanded).map(str::to_owned);
        out.push(DocBuild {
            recipe: recipe.clone(),
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

/// Numbers the probe directories so two of these can run side by side.
static PROBE_RUNS: AtomicU32 = AtomicU32::new(0);

/// Does rustdoc accept [`PROBE_SOURCE`] when given these flags? Run rather
/// than read: "`-D warnings` denies" is a claim about a tool, and every other
/// assertion in this file could pass on a Justfile whose flags were a typo.
fn rustdoc_accepts_the_probe(flags: &[&str]) -> bool {
    let run = PROBE_RUNS.fetch_add(1, Ordering::Relaxed);
    let dir = env::temp_dir().join(format!("aozora-md-doc-probe-{}-{run}", process::id()));
    let source = dir.join("probe.rs");
    fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("creating {}: {e}", dir.display()));
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

// --- the readers above, on the shapes that would fool them ------------------

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
