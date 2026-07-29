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
use std::fmt;
use std::fs;
use std::iter;
use std::mem;
use std::path::{Path, PathBuf};
use std::process::{self, Command};
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{Duration, Instant};

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

/// The tools the dev image installs, by package name.
///
/// `mise.toml` is the HOST copy — six tools, "latest", for editor integration
/// and the commit-msg hook. The image is where a tool becomes available to a
/// recipe, and it carries twenty-five. Reading only the first is how
/// `cargo-release` sat in tier D of the `Dockerfile` from the day that layer
/// was added, invoked by nothing, while the rule below reported every declared
/// tool as running.
///
/// Two forms, both read off the file rather than listed here. `cargo binstall`
/// and `cargo install` name their packages outright. Everything fetched from a
/// release archive is pinned by an `ARG <TOOL>_VERSION`, which is the same
/// declaration one layer up and the only one those `curl | tar` lines make in
/// a shape a reader can find — `just` is installed with its version inline and
/// is therefore the one tool in the image neither form sees.
fn image_tools(dockerfile: &str) -> BTreeSet<String> {
    let joined = dockerfile.replace("\\\n", " ");
    let mut out = BTreeSet::new();
    for line in joined.lines() {
        if let Some(rest) = line.strip_prefix("ARG ") {
            let name = rest.split('=').next().unwrap_or_default().trim();
            if let Some(tool) = name.strip_suffix("_VERSION") {
                out.insert(tool.to_ascii_lowercase().replace('_', "-"));
            }
            continue;
        }
        if !line.starts_with("RUN") {
            continue;
        }
        let body = strip_comment(line);
        let Some(at) = ["cargo binstall", "cargo install"]
            .iter()
            .find_map(|head| body.find(head).map(|at| at + head.len()))
        else {
            continue;
        };
        // Everything after the sub-command that is not a flag, a shell
        // expansion or a path is a package. `sccache@0.10.0` is a pin on one.
        for token in body[at..].split_whitespace() {
            if token.starts_with(['-', '$', '"', '\''])
                || token.contains('/')
                || token.contains('=')
            {
                continue;
            }
            out.insert(token.split('@').next().unwrap_or(token).to_owned());
        }
    }
    out
}

/// The words a package of that name can be invoked as. A `-cli` crate ships
/// the bare command (`typos-cli` → `typos`).
fn command_names(tool: &str) -> Vec<String> {
    let mut out = vec![tool.to_owned()];
    if let Some(stem) = tool.strip_suffix("-cli") {
        out.push(stem.to_owned());
    }
    out
}

/// The command lines this repo runs: the same text [`executed_words`] reduces
/// to a set, kept whole.
fn executed_lines(justfile: &str, lefthook: &str) -> Vec<String> {
    let mut out: Vec<String> = justfile
        .lines()
        .filter(|line| line.starts_with([' ', '\t']))
        .map(|line| strip_comment(line).to_owned())
        .collect();
    out.extend(
        lefthook
            .lines()
            .filter_map(|line| line.trim_start().strip_prefix("run:"))
            .map(str::to_owned),
    );
    out
}

/// Is `tool` invoked by anything this repo runs?
///
/// A cargo plug-in is reached as `cargo <sub>`, which is TWO ADJACENT WORDS,
/// and a set of words cannot tell them apart from the same two words far
/// apart. Measured on this repo: `just profile` names
/// `target/release/examples/samply_render`, which puts `release` in the
/// vocabulary of a Justfile that has never run `cargo release` — so the
/// one-word test reports the idle tool as running, which is the answer that
/// makes the rule useless for the one case it was widened for.
fn tool_is_invoked(tool: &str, lines: &[String], words: &BTreeSet<String>) -> bool {
    if let Some(sub) = tool.strip_prefix("cargo-") {
        let plugin = Regex::new(&format!(r"\bcargo(\s+\+\S+)?\s+{}\b", regex::escape(sub)))
            .unwrap_or_else(|e| panic!("compiling the reader for `{tool}`: {e}"));
        return lines
            .iter()
            .any(|line| plugin.is_match(line) || line.contains(tool));
    }
    command_names(tool).iter().any(|name| words.contains(name))
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

/// The lines nested under `key:` inside a block that is already indented —
/// a job's `permissions:`, say, out of the lines of that job.
///
/// The indentation is what makes it that block and not a neighbouring one:
/// `job_lines` hands back the whole job, steps included, and a reader that
/// went looking for a permission anywhere inside it would answer for a `with:`
/// value that happened to be spelled alike.
fn nested_block<'a>(lines: &[&'a str], key: &str) -> Vec<&'a str> {
    let header = format!("{key}:");
    let mut out = Vec::new();
    let mut opened_at = None;
    for line in lines {
        let body = strip_comment(line);
        if body.trim().is_empty() {
            continue;
        }
        let indent = body.len() - body.trim_start().len();
        match opened_at {
            Some(open) if indent > open => out.push(*line),
            Some(_) => break,
            None if body.trim() == header => opened_at = Some(indent),
            None => {}
        }
    }
    out
}

/// Is `key:` written in this block at all? Separate from what is under it,
/// because a job that declares `permissions:` REPLACES the workflow's grant
/// rather than adding to it — so an empty one is a statement, and the absence
/// of one is a different statement.
fn declares_key(lines: &[&str], key: &str) -> bool {
    let header = format!("{key}:");
    lines
        .iter()
        .any(|line| strip_comment(line).trim().starts_with(header.as_str()))
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

/// Tools the image installs that no recipe calls, each with the reason. Both
/// entries are debts rather than decisions, so both are written as such: a
/// name on this list is a layer the image pays for on every build and a
/// binary a reviewer will read as enforcement.
const INSTALLED_AND_NOT_RUN: &[(&str, &str)] = &[
    (
        "cargo-edit",
        "DEBT, and the only one on this list. Tier D of the Dockerfile installs \
         it beside cargo-release, and it is exactly what cargo-release was until `just \
         release` existed: an installed release helper nothing invokes. \
         `cargo add` and `cargo remove` are cargo's own since 1.62 and \
         `cargo release version` supersedes `cargo set-version`, so nothing is \
         known to need it. Left in place rather than removed here because \
         dropping a name from that layer is a Dockerfile change, not a test's.",
    ),
    (
        "node",
        "READER ARTIFACT, not a tool. `ARG NODE_VERSION` parameterises an apt \
         source URL in the node-base stage; what it installs is the JS RUNTIME \
         the image is built on, which bun and the playground toolchain reach \
         without any recipe naming `node` as a command.",
    ),
];

#[test]
fn every_tool_this_repo_declares_is_a_tool_this_repo_runs() {
    let justfile = read("Justfile");
    let lefthook = read("lefthook.yml");
    let executed = executed_words(&justfile, &lefthook);
    let lines = executed_lines(&justfile, &lefthook);

    let host = declared_tools(&read("mise.toml"));
    assert!(
        host.len() >= 5,
        "mise.toml yielded {} tools; the reader is not finding the `[tools]` table",
        host.len()
    );

    // The image is the other half, and the half the question is actually
    // about: a recipe can only call what the container holds, and `mise.toml`
    // holds six of the twenty-five names in the `Dockerfile`. Asking only the
    // small list is what let a release tool be installed, pinned, given its
    // own layer and called by nothing for as long as tier D has existed.
    let image = image_tools(&read("Dockerfile"));
    assert!(
        image.len() >= 20 && image.contains("cargo-nextest") && image.contains("vale"),
        "the Dockerfile came out installing {image:?}; the reader is not finding its \
         `cargo binstall` lists or its `ARG <TOOL>_VERSION` pins"
    );

    let mut idle: Vec<String> = host
        .iter()
        .chain(image.iter())
        .filter(|tool| {
            !INSTALLED_AND_NOT_RUN
                .iter()
                .any(|(excused, _)| excused == tool)
        })
        .filter(|tool| !tool_is_invoked(tool, &lines, &executed))
        .cloned()
        .collect();
    idle.sort();
    idle.dedup();
    assert!(
        idle.is_empty(),
        "installed by this repo and invoked nowhere: {idle:?}\n\
         A pinned, installed tool nothing calls reads as a gate and is not one. \
         Give it a `just` recipe (and a place in `just lint` / `just ci`), drop the \
         installation, or add it to INSTALLED_AND_NOT_RUN with the reason it is there."
    );
}

#[test]
fn the_image_shipped_a_release_tool_no_recipe_called() {
    // The `Justfile` as it stood: a `changelog` recipe that runs git-cliff and
    // a `dist-assets` recipe that regenerates the man page, and between them
    // no caller for the tool whose whole job is to move the version those two
    // describe. `cargo-release` is in the same `cargo binstall` list as
    // `cargo-edit`, one layer below `cargo-semver-checks`, and every rule in
    // this file passed on it — because the one that asks the question in the
    // right words was pointed at `mise.toml`, where it is not named.
    let dockerfile = concat!(
        "ARG ZIZMOR_VERSION=1.28.0\n",
        "RUN cargo binstall --no-confirm --locked --root /usr/local \\\n",
        "        cargo-semver-checks \\\n",
        "        sccache@0.10.0\n",
        "RUN cargo binstall --no-confirm --locked --root /usr/local \\\n",
        "        cargo-edit \\\n",
        "        cargo-release\n",
    );
    let tools = image_tools(dockerfile);
    assert_eq!(
        tools,
        [
            "cargo-edit",
            "cargo-release",
            "cargo-semver-checks",
            "sccache",
            "zizmor"
        ]
        .iter()
        .map(|name| (*name).to_owned())
        .collect::<BTreeSet<String>>(),
        "the reader no longer sees what the image installs; a pin, a continued list and a \
         version-suffixed package are all forms it has to read"
    );

    // The `Justfile` as it stood, including the line that made the obvious
    // reader answer wrongly: `just profile` names a path under
    // `target/release/`, and a word-set test cannot tell that from a call.
    let before = concat!(
        "changelog:\n",
        "    {{_dev}} git-cliff --unreleased\n",
        "profile REPEAT:\n",
        "    samply record -o /tmp/r.json.gz -- target/release/examples/samply_render {{REPEAT}}\n",
        "dist-assets:\n",
        "    {{_dev}} cargo run --package xtask -- gen-dist-assets\n",
    );
    let idle: Vec<&String> = tools
        .iter()
        .filter(|tool| {
            !tool_is_invoked(
                tool,
                &executed_lines(before, ""),
                &executed_words(before, ""),
            )
        })
        .collect();
    assert!(
        idle.contains(&&"cargo-release".to_owned()),
        "the reader stopped reporting the defect it was widened for: {idle:?}"
    );
    assert!(
        executed_words(before, "").contains("release"),
        "the fixture no longer holds the word that fools a one-word reader, so it is no longer \
         pinning the difference between this reader and that one"
    );

    let after = format!(
        "{before}release LEVEL:\n    {{{{_dev}}}} cargo release version {{{{LEVEL}}}} --workspace\n"
    );
    assert!(
        tool_is_invoked(
            "cargo-release",
            &executed_lines(&after, ""),
            &executed_words(&after, "")
        ),
        "a recipe that runs `cargo release <step>` is what settles it, and the reader does not \
         see one"
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

/// The recipes a workflow may run without being gates, each with the reason it
/// is not one. Anything else one runs is a check `just ci` does not run — the
/// original defect, in the direction the manifest does not close by itself.
const NOT_A_GATE_IN_A_WORKFLOW: &[(&str, &str)] = &[
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
    (
        "changelog-check",
        "not answerable on a branch, so not answerable by a gate: the section \
         it looks for is called `## [Unreleased]` until the release bump dates \
         it, so every pull request would fail it. publish-crates.yml's \
         preflight asks it of a release ref, which is the one ref where it has \
         an answer.",
    ),
];

#[test]
fn every_recipe_a_workflow_runs_is_a_gate_or_a_named_precondition() {
    // The other direction of the same drift, and the one derivation does not
    // close: nothing stops a `- run: just <something>` step being added back
    // to ci.yml. That is how `wasm-build` came to run in CI and not in
    // `just ci` — a check the workflow enforces that no local command
    // reproduces is exactly as broken as a check that runs nowhere.
    //
    // Over EVERY workflow, not ci.yml alone. The rule was written when ci.yml
    // was the only file that ran a recipe, and it has not been for a while:
    // audit.yml runs the two advisory gates on a cron, docs.yml builds the
    // rustdoc and the playground, and publish-crates.yml runs the preflight
    // that stands between a broken manifest and an irrevocable upload. That
    // last one is the file where an unaccounted recipe costs the most and the
    // file the rule reached least — it runs on `workflow_dispatch` alone, so a
    // step naming a recipe that does not exist first says so on release day.
    let justfile = read("Justfile");
    let manifest = recipes_in_group(&justfile, "gate");
    let mut invoked: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    for path in workflow_files() {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        for recipe in recipes_invoked(&text) {
            invoked.entry(recipe).or_default().insert(label.clone());
        }
    }

    assert!(
        invoked.len() >= 6,
        "the workflows came out running {invoked:?}; the reader is not finding their `just` steps"
    );
    let unaccounted: Vec<String> = invoked
        .iter()
        .filter(|(recipe, _)| {
            !manifest.contains(*recipe)
                && !NOT_A_GATE_IN_A_WORKFLOW
                    .iter()
                    .any(|(allowed, _)| allowed == recipe)
        })
        .map(|(recipe, sites)| format!("{recipe} (in {sites:?})"))
        .collect();
    assert!(
        unaccounted.is_empty(),
        "these run in a workflow and `[group('gate')]` does not declare them: {unaccounted:?}\n\
         Tag the recipe so `just ci` runs it too, or add it to NOT_A_GATE_IN_A_WORKFLOW with the \
         reason it is not a gate."
    );

    // A recipe a workflow names and the Justfile does not have is a step that
    // cannot start. In ci.yml that costs a pull request; in the release path
    // it costs the dispatch somebody made to cut a release.
    for (recipe, sites) in &invoked {
        assert!(
            recipe_exists(&justfile, recipe),
            "{sites:?} run `just {recipe}` and the Justfile has no such recipe"
        );
    }
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
    // `vale` joins them for the same reason and with the same exposure: it is
    // discussed at length in prose two recipes apart, and a `vale` recipe that
    // stopped running the binary would leave every prose assertion in this
    // file checking a configuration nothing reads.
    for tool in [
        "zizmor",
        "actionlint",
        "typos",
        "lefthook",
        "committed",
        "vale",
    ] {
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

/// Is this call the upload rather than the packaging? A `cargo publish` is
/// both commands under one name, and only one of them is a build: with
/// `--dry-run` it packages and verify-builds every selected crate, which is
/// what `just package` owns, and without it the same run ends by pushing to
/// crates.io. No recipe in this repo may do that — a gate is something anyone
/// can run — so the upload is the one `cargo publish` a workflow has to spell
/// out for itself, and the rule below has to let it.
///
/// The flag is looked for after `cargo publish` and not anywhere on the line,
/// which is the difference between reading the command and reading the text
/// around it: `echo --dry-run && cargo publish --locked` is an upload.
fn uploads_to_a_registry(line: &str, tool: &str, sub: &str) -> bool {
    if tool != "cargo" || sub != "publish" {
        return false;
    }
    let tokens = shell_tokens(line);
    let Some(at) = publish_at(&tokens) else {
        return false;
    };
    !tokens[at..].iter().any(|token| token == "--dry-run")
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
    // The prefixes are read off the file rather than listed here. A list is
    // the same defect one level down: `_pg_install` was added to the Justfile
    // and to this list separately, and a fifth prefix added to only one of the
    // two would make every recipe using it invisible to the rule below —
    // which is the rule that says a job outside the dev image has to announce
    // itself.
    let markers: Vec<String> = container_prefixes(justfile)
        .keys()
        .map(|name| format!("{{{{{name}}}}}"))
        .collect();
    recipe_body(justfile, recipe).iter().any(|line| {
        let body = strip_comment(line);
        body.contains("docker compose")
            || markers.iter().any(|marker| body.contains(marker.as_str()))
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

/// Workflow steps that write a build out themselves, each with the reason it
/// cannot call a recipe instead. A row excuses one `<tool> <sub-command>` in
/// one file, from both halves of the rule below.
///
/// Empty, and that is the finished state rather than a table nobody has filled
/// in. It held three rows, all in `docs.yml`: the wasm-pack build, the bun
/// install and the bun build behind `just playground-build`. The reason each
/// gave was the same one — the `playground*` recipes hard-coded `docker
/// compose run`, so unlike `just doc` they could not run on the Pages runner.
/// DEV-310 put those two prefixes through the Justfile's `_in` switch, which
/// makes the reason false, so the rows go rather than stay as prose that no
/// longer describes anything. The mechanism stays: a future step that genuinely
/// cannot reach its recipe adds a row and says why.
const RE_SPELLED_BUILD: &[(&str, &str, &str, &str)] = &[];

/// The two things that can be wrong with a build a workflow writes out, per
/// workflow. Both are the same defect counted from opposite ends — a command
/// whose one definition is not the Justfile's — so one walk answers both.
#[derive(Default)]
struct WorkflowBuilds {
    /// A gate recipe defines this command and the workflow spelled it again.
    re_spelled: Vec<String>,
    /// No gate recipe defines it at all, so the only thing that ever runs it
    /// is this workflow, on this workflow's triggers.
    ungated: Vec<String>,
    /// Commands looked at, exemptions included. Both lists above are empty in
    /// the finished state, which is also what a reader that has stopped
    /// finding commands reports — so what the reader saw is counted too.
    examined: usize,
}

/// Every build one workflow drives, sorted into those two.
fn workflow_builds(
    label: &str,
    text: &str,
    owned: &BTreeMap<(String, String), String>,
) -> WorkflowBuilds {
    let mut out = WorkflowBuilds::default();
    for line in jobs_block(text) {
        let body = strip_comment(line);
        for (tool, sub) in tool_commands(body) {
            out.examined += 1;
            if uploads_to_a_registry(body, &tool, &sub)
                || RE_SPELLED_BUILD
                    .iter()
                    .any(|&(file, owner, name, _)| file == label && owner == tool && name == sub)
            {
                continue;
            }
            let written = body.trim();
            match owned.get(&(tool.clone(), sub.clone())) {
                Some(gate) => out.re_spelled.push(format!(
                    "{label}: `{tool} {sub}` is the build `just {gate}` defines\n      {written}"
                )),
                None => out.ungated.push(format!(
                    "{label}: `{tool} {sub}` is a build no `[group('gate')]` recipe \
                     defines\n      {written}"
                )),
            }
        }
    }
    out
}

#[test]
fn every_build_a_workflow_drives_is_defined_once_and_by_a_gate() {
    // `the_workflow_hand_writes_no_gate_of_its_own` asks this of ci.yml, by
    // NAME. Both narrowings mattered: the duplicate lived in docs.yml, and it
    // never named the `doc` gate — it wrote the gate's command out instead.
    // What a check is, is the command it runs, so that is what has to be
    // single-sourced.
    //
    // The half below `ungated` is the other end of that sentence, and it was
    // missing until DEV-224. The rule read a workflow's builds and asked only
    // "does a gate already define this one" — so a build with TWO definitions
    // failed and a build with NONE passed, silently, because `owned.get`
    // returned `None` and the loop moved on. The command that lived in that
    // blind spot was `publish-crates.yml`'s `cargo publish --workspace
    // --dry-run --locked`: the one build in this repo that compiles the
    // crates as a consumer receives them, in a workflow whose only trigger is
    // `workflow_dispatch`. Nothing in front of a merge ran it, and the rule
    // whose title is "one definition of a build, wherever it is written" was
    // looking straight at it — a build defined nowhere is not one definition,
    // and it is the worse of the two failures, because a second copy at least
    // runs.
    let justfile = read("Justfile");
    let owned = commands_owned_by_gates(&justfile);
    assert!(
        owned.len() >= 8,
        "the gates came out running {owned:?}; the reader is not finding their bodies"
    );

    let mut found = WorkflowBuilds::default();
    for path in workflow_files() {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        let builds = workflow_builds(&label, &text, &owned);
        found.re_spelled.extend(builds.re_spelled);
        found.ungated.extend(builds.ungated);
        found.examined += builds.examined;
    }
    // Every build in this repo is a recipe's now, so both lists below are
    // meant to be empty — which is also what this rule reports if the reader
    // has gone blind. The one command still written where it runs is the
    // upload, so the floor is one: past it, silence is a finding.
    assert!(
        found.examined >= 1,
        "no workflow came out running any of {BUILD_TOOLS:?}, not even the crates.io upload. \
         The reader is finding nothing, and a reader finding nothing passes every workflow."
    );
    assert!(
        found.re_spelled.is_empty(),
        "workflow steps that re-spell a gate's build:\n{}\n\
         Run the recipe instead, or add the step to RE_SPELLED_BUILD with the reason it cannot — \
         a second copy drifts from the first in whichever direction nobody is looking.",
        found.re_spelled.join("\n")
    );
    assert!(
        found.ungated.is_empty(),
        "workflow steps that build something no gate builds:\n{}\n\
         A build only a workflow defines runs on that workflow's triggers and nowhere else, so \
         whatever it would have caught is caught at whatever moment that workflow happens to \
         fire — for a `workflow_dispatch` file, the moment somebody decides to release. Give the \
         command a `[group('gate')]` recipe and call it from here, or add the step to \
         RE_SPELLED_BUILD with the reason it cannot be one.",
        found.ungated.join("\n")
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
// one command line, both sides of the `_in` switch
// ---------------------------------------------------------------------------
//
// The rule above holds a workflow to saying where it is. It never held the
// Justfile to listening, and for the `playground*` recipes it could not have:
// `_pg` and `_pg_install` were plain `docker compose run` strings, so a job
// could set `AOZORA_MD_IN_CONTAINER=1`, call `just playground-build`, and
// still be handed a `docker compose run` on a runner with no daemon to serve
// it. docs.yml never hit that only because it wrote the three commands out by
// hand instead — which is the entry `RE_SPELLED_BUILD` above used to carry,
// giving that exact reason. One defect, written down twice as an exemption
// and checked in neither place.
//
// A switch is two halves: the caller says where it is, and every prefix
// collapses to nothing when it does. This section is the second half, plus
// the other thing a command inherits from its surroundings and nothing
// compared — the directory it starts in.

/// The environment variable a caller sets to say it is already inside one of
/// the images. Workflows set it; the `Justfile` reads it once, into the
/// variable every run prefix branches on.
const IN_CONTAINER_ENV: &str = "AOZORA_MD_IN_CONTAINER";

/// Every `NAME := <value>` assignment, right-hand side as written.
///
/// Unlike [`plain_variables`], an `if` expression is kept rather than skipped:
/// the shape of that expression is the entire question here.
fn assignments(justfile: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for line in justfile.lines() {
        if line.starts_with([' ', '\t', '#', '[']) {
            continue;
        }
        let Some((name, value)) = line.split_once(":=") else {
            continue;
        };
        out.push((name.trim().to_owned(), value.trim().to_owned()));
    }
    out
}

/// The `Justfile` variable holding the answer to "am I already inside the
/// image": the one assigned from [`IN_CONTAINER_ENV`].
fn in_container_variable(justfile: &str) -> String {
    assignments(justfile)
        .into_iter()
        .find(|(_, value)| value.contains("env_var_or_default") && value.contains(IN_CONTAINER_ENV))
        .map_or_else(
            || {
                panic!(
                    "no Justfile variable is assigned from {IN_CONTAINER_ENV} any more. Every run \
                     prefix branches on it and every workflow outside the dev image sets it, so a \
                     rename has to reach all three at once."
                )
            },
            |(name, _)| name,
        )
}

/// Every variable whose value puts a command inside a container.
fn container_prefixes(justfile: &str) -> BTreeMap<String, String> {
    assignments(justfile)
        .into_iter()
        .filter(|(_, value)| value.contains("docker compose run"))
        .collect()
}

/// Does this right-hand side become nothing when the switch says the caller is
/// already inside the image?
///
/// Textual, and deliberately so. The property is that the `"1"` branch is the
/// empty string; reading it off the source is what makes an inverted switch,
/// or one that merely mentions the variable, fail instead of pass.
fn collapses_inside_the_image(value: &str, switch: &str) -> bool {
    let switched = format!("if {switch} == \"1\" {{ \"\" }} else {{");
    value.trim_start().starts_with(&switched)
}

#[test]
fn every_prefix_that_enters_a_container_is_one_the_env_var_empties() {
    let justfile = read("Justfile");
    let switch = in_container_variable(&justfile);
    let prefixes = container_prefixes(&justfile);
    assert!(
        prefixes.len() >= 4,
        "only {} run prefix(es) came out spelling `docker compose run`; the reader is not finding \
         them: {prefixes:?}",
        prefixes.len()
    );

    let unswitched: Vec<String> = prefixes
        .iter()
        .filter(|(_, value)| !collapses_inside_the_image(value, &switch))
        .map(|(name, value)| format!("{name} := {value}"))
        .collect();
    assert!(
        unswitched.is_empty(),
        "these run prefixes enter a container unconditionally:\n  {}\n\
         `{IN_CONTAINER_ENV}=1` is a caller saying it is already inside an image — a devcontainer, \
         a `just shell`, docs.yml's Pages job, ci.yml's native gates. A prefix that ignores it \
         makes every recipe built on it unrunnable from all of those, and the workaround is one \
         this repo has already paid for once: the workflow gives up and writes the recipe's \
         command out by hand.",
        unswitched.join("\n  ")
    );
}

const COMPOSE_FILE: &str = "docker-compose.yml";

/// The lines of each service in the compose file, keyed by service name.
///
/// Only the `services:` mapping. `x-common-env` and the top-level `volumes:`
/// hold keys indented alike, so the column-zero header above a line is what
/// says which block it is in.
fn compose_services(compose: &str) -> BTreeMap<String, Vec<&str>> {
    let mut out: BTreeMap<String, Vec<&str>> = BTreeMap::new();
    let mut current: Option<String> = None;
    let mut inside = false;
    for line in compose.lines() {
        // A blank line separates two services without ending the mapping;
        // reading it as a column-zero key would end the walk at the first one.
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with([' ', '\t']) {
            inside = line.starts_with("services:");
            current = None;
            continue;
        }
        if !inside {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        if line.len() - trimmed.len() == 2 {
            current = trimmed.strip_suffix(':').map(str::to_owned);
            if let Some(name) = &current {
                out.entry(name.clone()).or_default();
            }
            continue;
        }
        if let Some(name) = &current {
            out.entry(name.clone()).or_default().push(line);
        }
    }
    out
}

/// The value of a scalar key inside a service block.
fn service_value<'a>(lines: &[&'a str], key: &str) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        strip_comment(line)
            .trim()
            .strip_prefix(key)?
            .strip_prefix(':')
            .map(str::trim)
    })
}

/// Where a service mounts the repository: the `volumes:` entry whose source is
/// `.`, with any `:cached` / `:ro` option dropped.
fn repo_mount<'a>(lines: &[&'a str]) -> Option<&'a str> {
    lines.iter().find_map(|line| {
        let entry = strip_comment(line).trim().strip_prefix("- ")?;
        let target = entry.strip_prefix(".:")?;
        Some(target.split(':').next().unwrap_or(target))
    })
}

#[test]
fn every_service_starts_a_command_where_just_would_have_started_it() {
    // The other half of "one Justfile, both worlds". A recipe body is a single
    // command line and the switch above decides only whether a container is
    // wrapped around it — never what it says. So the cwd has to be the same on
    // both sides, and outside a container it is wherever `just` ran, i.e. the
    // repo root.
    //
    // The `playground` service started at `/workspace/playground` instead, and
    // that is what made its recipes un-switchable: they were written for a cwd
    // only the container had, so there was no single command line to switch.
    // The `cd playground` each of them now spells is the fix, and this is what
    // keeps the compose file from re-introducing the assumption it replaced.
    let compose = read(COMPOSE_FILE);
    let services = compose_services(&compose);
    assert!(
        services.len() >= 4,
        "only {} service(s) came out of {COMPOSE_FILE}; the reader is not finding them",
        services.len()
    );

    let mut wrong = Vec::new();
    for (name, lines) in &services {
        let Some(mount) = repo_mount(lines) else {
            wrong.push(format!("{name}: mounts the repository nowhere"));
            continue;
        };
        match service_value(lines, "working_dir") {
            Some(dir) if dir == mount => {}
            Some(dir) => wrong.push(format!(
                "{name}: starts at `{dir}`, but the repository is mounted at `{mount}`"
            )),
            None => wrong.push(format!("{name}: declares no working_dir")),
        }
    }
    assert!(
        wrong.is_empty(),
        "these services do not start where `just` does:\n  {}\n\
         A recipe reaches its tool through a prefix that may be nothing at all, so the same text \
         has to mean the same thing with and without a container around it. Give the recipe its \
         own `cd`, the way the `fuzz*` ones do, rather than the service a working_dir only one \
         side of the switch will see.",
        wrong.join("\n  ")
    );
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn a_prefix_reads_as_switched_only_when_the_env_var_empties_it() {
    assert!(
        collapses_inside_the_image(
            r#"if _in == "1" { "" } else { "docker compose run --rm dev" }"#,
            "_in"
        ),
        "the shape every prefix in this Justfile is written in did not read as switched"
    );
    assert!(
        !collapses_inside_the_image(
            r#""docker compose run --rm --service-ports playground""#,
            "_in"
        ),
        "the unconditional spelling `_pg` carried before DEV-310 read as switched"
    );
    assert!(
        !collapses_inside_the_image(
            r#"if _in == "1" { "docker compose run --rm dev" } else { "" }"#,
            "_in"
        ),
        "an inverted switch read as a switch — naming the variable is not obeying it"
    );
    assert!(
        !collapses_inside_the_image(
            r#"if _in == "0" { "" } else { "docker compose run --rm dev" }"#,
            "_in"
        ),
        "a switch comparing against the wrong value read as correct"
    );
}

#[test]
fn a_compose_key_is_read_from_its_own_service_and_not_a_neighbour() {
    let compose = concat!(
        "x-common-env: &common-env\n",
        "  WORKING_DIR: /elsewhere\n",
        "services:\n",
        "  dev:\n",
        "    working_dir: /workspace\n",
        "    volumes:\n",
        "      - .:/workspace:cached\n",
        "      - cargo-target:/cargo/target\n",
        "  playground:\n",
        "    working_dir: /workspace/playground  # the shape this test rejects\n",
        "    volumes:\n",
        "      - .:/workspace\n",
        "volumes:\n",
        "  cargo-target:\n",
    );
    let services = compose_services(compose);
    assert_eq!(
        services.keys().collect::<Vec<_>>(),
        ["dev", "playground"],
        "a key outside the `services:` mapping was read as a service"
    );
    let dev = &services["dev"];
    assert_eq!(
        service_value(dev, "working_dir"),
        Some("/workspace"),
        "a service's own working_dir was not found"
    );
    assert_eq!(
        repo_mount(dev),
        Some("/workspace"),
        "the repo mount was not read past its `:cached` option"
    );
    assert_eq!(
        repo_mount(dev).map(str::to_owned),
        service_value(dev, "working_dir").map(str::to_owned),
        "the two readers disagree about a service that is correct"
    );
    let playground = &services["playground"];
    assert_ne!(
        service_value(playground, "working_dir"),
        repo_mount(playground),
        "the arrangement that stood before DEV-310 read as correct; the trailing comment on the \
         working_dir line is the shape that would hide it"
    );
}

// ---------------------------------------------------------------------------
// what the two new playground gates actually reach
// ---------------------------------------------------------------------------
//
// `playground-lint` and `playground-test` are the first gates over the
// TypeScript tree, and both are the shape this file keeps finding: a check
// whose reach is a glob list in a config file nobody compares to the tree.
// `tsc --noEmit` was the whole of the static analysis over ~15 modules, so
// there is no second net under these — an `includes` narrowed to
// `src/**/*.ts` would silently drop all eight `.tsx` components and the gate
// would stay green, which is indistinguishable from the tree being clean.
//
// Same for the empty answer. `biome check` exits 0 on a warn-level rule and
// `vitest run` is one flag away from exiting 0 on no tests at all; a gate
// that cannot fail on the absence of what it checks is the `doc` gate's
// defect in a different language.

/// A `**` / `*` glob as a regex over a `/`-separated path. The subset biome's
/// `files.includes` and vitest's `test.include` are written in.
fn glob_to_regex(pattern: &str) -> Regex {
    let mut out = String::from("^");
    let mut chars = pattern.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '*' if chars.peek() == Some(&'*') => {
                chars.next();
                // `**/` spans any number of directories INCLUDING none, so
                // `src/**/*.ts` has to match `src/outline.ts`. A `**` not
                // followed by a separator is the unanchored form.
                if chars.peek() == Some(&'/') {
                    chars.next();
                    out.push_str("(?:[^/]+/)*");
                } else {
                    out.push_str(".*");
                }
            }
            '*' => out.push_str("[^/]*"),
            '?' => out.push_str("[^/]"),
            '{' | '}' | ',' => match ch {
                '{' => out.push('('),
                '}' => out.push(')'),
                _ => out.push('|'),
            },
            _ => out.push_str(&regex::escape(&ch.to_string())),
        }
    }
    out.push('$');
    Regex::new(&out)
        .unwrap_or_else(|e| panic!("`{pattern}` is not a glob this reader can read: {e}"))
}

const PLAYGROUND: &str = "playground";

/// Every file under `playground/` with one of `extensions`, as a path
/// relative to that directory. Build output and the dependency tree are not
/// authored here and no gate is asked to read them.
fn playground_files(extensions: &[&str]) -> BTreeSet<String> {
    fn walk(dir: &Path, root: &Path, extensions: &[&str], out: &mut BTreeSet<String>) {
        let entries =
            fs::read_dir(dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if matches!(name.as_str(), "node_modules" | "dist" | ".vite") {
                    continue;
                }
                walk(&path, root, extensions, out);
                continue;
            }
            if extensions.iter().any(|ext| name.ends_with(ext)) {
                let relative = path.strip_prefix(root).unwrap_or(&path);
                out.insert(relative.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    let root = repo_root().join(PLAYGROUND);
    let mut out = BTreeSet::new();
    walk(&root, &root, extensions, &mut out);
    out
}

/// The `files.includes` list from `biome.json`, split into the patterns that
/// admit a file and the `!`-prefixed ones that take it back out.
fn biome_net() -> (Vec<String>, Vec<String>) {
    let text = read("playground/biome.json");
    let config: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("playground/biome.json: {e}"));
    let entries = config["files"]["includes"]
        .as_array()
        .unwrap_or_else(|| panic!("playground/biome.json declares no `files.includes` any more"));
    let mut admitted = Vec::new();
    let mut excluded = Vec::new();
    for entry in entries {
        let pattern = entry.as_str().unwrap_or_default();
        match pattern.strip_prefix('!') {
            Some(rest) => excluded.push(rest.to_owned()),
            None => admitted.push(pattern.to_owned()),
        }
    }
    (admitted, excluded)
}

/// The `include` list of `vite.config.ts`'s `test` block.
fn vitest_include() -> Vec<String> {
    let config = read("playground/vite.config.ts");
    // Anchored on the `test:` block. `build.rollupOptions` is a sibling with
    // list-valued keys of its own, and a reader that took the first `include:`
    // in the file would be reporting on whichever block moved above it.
    let (_, after) = config
        .split_once("\n  test: {")
        .unwrap_or_else(|| panic!("vite.config.ts has no `test` block any more"));
    let list = Regex::new(r"include:\s*\[([^\]]*)\]")
        .expect("a literal regex")
        .captures(after)
        .unwrap_or_else(|| panic!("vite.config.ts's `test` block declares no `include`"));
    list[1]
        .split(',')
        .map(|item| item.trim().trim_matches('\'').trim_matches('"').to_owned())
        .filter(|item| !item.is_empty())
        .collect()
}

#[test]
fn every_typescript_file_in_the_playground_is_one_the_lint_gate_reads() {
    let (patterns, negations) = biome_net();
    let admits: Vec<Regex> = patterns.iter().map(|p| glob_to_regex(p)).collect();
    let refuses: Vec<Regex> = negations.iter().map(|p| glob_to_regex(p)).collect();

    let sources = playground_files(&[".ts", ".tsx"]);
    assert!(
        sources.len() >= 20,
        "only {} TypeScript file(s) found under {PLAYGROUND}/; the reader is not finding the tree",
        sources.len()
    );

    let unread: Vec<&String> = sources
        .iter()
        .filter(|path| {
            !admits.iter().any(|re| re.is_match(path))
                || refuses.iter().any(|re| {
                    path.split('/')
                        .scan(String::new(), |at, segment| {
                            if !at.is_empty() {
                                at.push('/');
                            }
                            at.push_str(segment);
                            Some(at.clone())
                        })
                        .any(|prefix| re.is_match(&prefix))
                })
        })
        .collect();
    assert!(
        unread.is_empty(),
        "biome's `files.includes` does not reach these:\n  {}\n\
         `just playground-lint` passes on a file it never opened, which reads exactly like the \
         file being clean. Widen `files.includes` in playground/biome.json.\n\
         admitted: {patterns:?}\n  refused: {negations:?}",
        unread
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

#[test]
fn every_test_file_beside_a_playground_module_is_one_the_test_gate_runs() {
    // The narrowest net here, and the one with a live gap: `src/**/*.test.ts`
    // does not match `.test.tsx`. A Solid component test would sit beside its
    // component, be committed, be reviewed, and never run — and `vitest run`
    // would report the suite it did run as passing.
    let include: Vec<Regex> = vitest_include().iter().map(|p| glob_to_regex(p)).collect();
    let tests: BTreeSet<String> = playground_files(&[".ts", ".tsx"])
        .into_iter()
        .filter(|path| path.contains(".test."))
        .collect();
    assert!(
        tests.len() >= 4,
        "only {} test file(s) found under {PLAYGROUND}/; the reader is not finding them",
        tests.len()
    );

    let unrun: Vec<&String> = tests
        .iter()
        .filter(|path| !include.iter().any(|re| re.is_match(path)))
        .collect();
    assert!(
        unrun.is_empty(),
        "vitest's `include` does not match these:\n  {}\n\
         They are test files that no gate runs. Widen the `include` list in the `test` block of \
         playground/vite.config.ts.",
        unrun
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

/// A playground gate, the `package.json` script it runs, and what that script
/// must and must not say for the gate to be able to fail.
///
/// Both entries are the same defect in two tools: the default is to report,
/// and reporting exits 0.
const PLAYGROUND_SCRIPT_POLICY: &[(&str, &str, &[&str], &[&str])] = &[
    (
        "playground-lint",
        "lint",
        // Several of biome's recommended rules are warn-level, and `biome
        // check` exits 0 on those.
        &["--error-on-warnings"],
        &[],
    ),
    (
        "playground-test",
        "test",
        &[],
        // With it, a suite that matched nothing — a renamed `include`, a
        // deleted directory — is a green gate.
        &["--passWithNoTests"],
    ),
];

#[test]
fn a_playground_gate_fails_on_the_absence_of_what_it_checks() {
    let justfile = read("Justfile");
    let text = read("playground/package.json");
    let manifest: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("playground/package.json: {e}"));

    for &(recipe, script, required, forbidden) in PLAYGROUND_SCRIPT_POLICY {
        assert!(
            recipes_in_group(&justfile, "gate").contains(recipe),
            "`{recipe}` is not in the gate manifest any more"
        );
        let body = recipe_body(&justfile, recipe).join(" ");
        assert!(
            body.contains(&format!("bun run {script}")),
            "`just {recipe}` no longer runs `bun run {script}`, so the policy below is being \
             checked against a script nothing calls"
        );

        let command = manifest["scripts"][script]
            .as_str()
            .unwrap_or_else(|| panic!("playground/package.json declares no `{script}` script"));
        for flag in required {
            assert!(
                command.contains(flag),
                "the `{script}` script (`{command}`) does not pass `{flag}`. Without it the tool \
                 reports its findings and exits 0, so `just {recipe}` is a report rather than a \
                 gate."
            );
        }
        for flag in forbidden {
            assert!(
                !command.contains(flag),
                "the `{script}` script (`{command}`) passes `{flag}`, which makes `just {recipe}` \
                 pass when it checked nothing at all."
            );
        }
    }
}

#[test]
fn every_bun_script_a_recipe_runs_is_one_the_playground_declares() {
    // The binding the two tests above lean on, asked of every recipe rather
    // than of the two gates: `bun run <name>` is resolved by package.json, and
    // a renamed script fails at run time in a container, on a line no reader
    // of the Justfile can see is wrong.
    let justfile = read("Justfile");
    let text = read("playground/package.json");
    let manifest: Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("playground/package.json: {e}"));
    let declared: BTreeSet<&str> = manifest["scripts"]
        .as_object()
        .unwrap_or_else(|| panic!("playground/package.json declares no scripts"))
        .keys()
        .map(String::as_str)
        .collect();

    let called = Regex::new(r"bun run ([a-z][a-z0-9:-]*)").expect("a literal regex");
    let mut missing = Vec::new();
    let mut seen = 0;
    for line in justfile.lines() {
        if !line.starts_with([' ', '\t']) {
            continue;
        }
        for capture in called.captures_iter(strip_comment(line)) {
            seen += 1;
            let script = &capture[1];
            if !declared.contains(script) {
                missing.push(format!("`bun run {script}` — {}", line.trim()));
            }
        }
    }
    assert!(
        seen >= 4,
        "only {seen} `bun run` call(s) found in the Justfile; the reader is not finding them"
    );
    assert!(
        missing.is_empty(),
        "these recipes call a script playground/package.json does not declare:\n  {}",
        missing.join("\n  ")
    );
}

/// A lefthook `glob:` as a regex. A THIRD dialect, and the difference is the
/// whole reason this reader is separate from [`glob_to_regex`]: lefthook's
/// `**` spans one directory or many, never none, so `a/**/*.ts` does not match
/// `a/b.ts` — where biome's and vitest's identically-spelled pattern does.
///
/// Measured, not assumed, against the pinned lefthook 2.1.9:
/// `lefthook run pre-commit --command playground --file <path>` reports
/// `(skip) no matching staged files` for `playground/vite.config.ts` and runs
/// for `playground/src/outline.ts`.
fn lefthook_glob_to_regex(pattern: &str) -> Regex {
    let spanning = glob_to_regex(pattern)
        .as_str()
        .replace("(?:[^/]+/)*", "(?:[^/]+/)+");
    Regex::new(&spanning).unwrap_or_else(|e| panic!("`{pattern}` is not a glob: {e}"))
}

/// The `pre-commit` commands lefthook declares: for each, the `glob:` patterns
/// gating it and the command line it runs.
fn pre_commit_commands(lefthook: &str) -> BTreeMap<String, (Vec<String>, Option<String>)> {
    let mut out: BTreeMap<String, (Vec<String>, Option<String>)> = BTreeMap::new();
    let mut in_hook = false;
    let mut in_commands = false;
    let mut current: Option<String> = None;
    let mut in_glob = false;
    for line in lefthook.lines() {
        if line.trim().is_empty() {
            continue;
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with('#') {
            continue;
        }
        let indent = line.len() - trimmed.len();
        if indent == 0 {
            in_hook = line.starts_with("pre-commit:");
            in_commands = false;
            current = None;
            continue;
        }
        if !in_hook {
            continue;
        }
        if indent == 2 {
            in_commands = trimmed.starts_with("commands:");
            current = None;
            continue;
        }
        if !in_commands {
            continue;
        }
        if indent == 4 {
            current = trimmed.strip_suffix(':').map(str::to_owned);
            in_glob = false;
            if let Some(name) = &current {
                out.entry(name.clone()).or_default();
            }
            continue;
        }
        let Some(name) = current.clone() else {
            continue;
        };
        let entry = out.entry(name).or_default();
        if let Some(value) = trimmed.strip_prefix("glob:") {
            in_glob = true;
            let value = value.trim();
            if !value.is_empty() {
                entry.0.push(value.trim_matches('"').to_owned());
                in_glob = false;
            }
        } else if let Some(value) = trimmed.strip_prefix("- ") {
            // Only inside the list `glob:` opened; `tags:` has one too.
            if in_glob {
                entry.0.push(value.trim().trim_matches('"').to_owned());
            }
        } else if let Some(value) = trimmed.strip_prefix("run:") {
            in_glob = false;
            entry.1 = Some(value.trim().to_owned());
        } else {
            in_glob = false;
        }
    }
    out
}

#[test]
fn the_pre_commit_hook_over_the_playground_fires_on_every_file_its_gate_reads() {
    // A hook behind a glob is two decisions — which check, and on what — and
    // only the first one is visible when it is right. This one was wrong in
    // the direction that matters most: the files it skipped were the gate's
    // own configuration, so a commit that switched the gate off was a commit
    // it did not look at.
    let lefthook = read("lefthook.yml");
    let commands = pre_commit_commands(&lefthook);
    let gate = "just playground-lint";
    let (globs, _) = commands
        .values()
        .find(|(_, run)| run.as_deref() == Some(gate))
        .unwrap_or_else(|| {
            panic!("no pre-commit command runs `{gate}` any more; the hook this checks is gone")
        });
    assert!(
        !globs.is_empty(),
        "the `{gate}` hook has no glob; it now runs on every commit, which is a different \
         decision from the one recorded in lefthook.yml"
    );
    let fires: Vec<Regex> = globs.iter().map(|p| lefthook_glob_to_regex(p)).collect();

    // What the gate reads: biome's own net, resolved against the tree.
    let (patterns, _) = biome_net();
    let admits: Vec<Regex> = patterns.iter().map(|p| glob_to_regex(p)).collect();
    let read_by_the_gate: Vec<String> = playground_files(&[".ts", ".tsx", ".json"])
        .into_iter()
        .filter(|path| admits.iter().any(|re| re.is_match(path)))
        .map(|path| format!("{PLAYGROUND}/{path}"))
        .collect();
    assert!(
        read_by_the_gate.len() >= 25,
        "only {} file(s) came out as read by the gate; the reader is not finding the tree",
        read_by_the_gate.len()
    );

    let skipped: Vec<&String> = read_by_the_gate
        .iter()
        .filter(|path| !fires.iter().any(|re| re.is_match(path)))
        .collect();
    assert!(
        skipped.is_empty(),
        "`just playground-lint` reads these and the pre-commit hook does not fire on them:\n  {}\n\
         Editing one and committing runs no check at all, and `just ci` at push time is the only \
         thing left. lefthook's `**` needs at least one directory under it, so a top-level file \
         wants its own pattern.\n  globs: {globs:?}",
        skipped
            .iter()
            .map(|path| path.as_str())
            .collect::<Vec<_>>()
            .join("\n  ")
    );
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn the_two_glob_dialects_differ_where_they_were_measured_to_differ() {
    // Held apart on purpose. Reading one file's globs with the other's rules
    // is how the hook came to look like it covered a tree it did not.
    for (pattern, path) in [
        ("playground/**/*.{ts,tsx,json}", "playground/vite.config.ts"),
        ("src/**/*.ts", "src/outline.ts"),
    ] {
        assert!(
            glob_to_regex(pattern).is_match(path),
            "biome/vitest spell `**/` as spanning zero directories; `{pattern}` missed `{path}`"
        );
        assert!(
            !lefthook_glob_to_regex(pattern).is_match(path),
            "lefthook's `**` needs a directory under it; `{pattern}` was read as matching `{path}`, \
             which is the reading that hid the gap this test exists for"
        );
    }
    // And where they agree, they agree.
    for (pattern, path) in [
        ("playground/**/*.{ts,tsx,json}", "playground/src/outline.ts"),
        ("playground/*.{ts,json}", "playground/package.json"),
    ] {
        assert!(
            lefthook_glob_to_regex(pattern).is_match(path),
            "`{pattern}` should fire on `{path}`"
        );
    }
    assert!(
        !lefthook_glob_to_regex("playground/*.{ts,json}").is_match("README.md"),
        "a playground glob matched a file outside the playground"
    );
}

#[test]
fn a_hook_glob_is_read_off_its_own_command_and_not_a_neighbouring_list() {
    let lefthook = concat!(
        "pre-commit:\n",
        "  commands:\n",
        "    fmt:\n",
        "      glob: \"*.rs\"\n",
        "      run: just fmt\n",
        "    playground:\n",
        "      glob:\n",
        "        - \"playground/*.{ts,json}\"\n",
        "        - \"playground/**/*.{ts,tsx,json}\"\n",
        "      run: just playground-lint\n",
        "      fail_text: |\n",
        "        - not a glob, and neither is the run: word in this sentence\n",
        "    typos:\n",
        "      run: just typos\n",
        "pre-push:\n",
        "  commands:\n",
        "    deep:\n",
        "      glob: \"*.never\"\n",
        "      run: just prop-deep\n",
        "      tags:\n",
        "        - deep\n",
    );
    let commands = pre_commit_commands(lefthook);
    assert_eq!(
        commands.keys().collect::<Vec<_>>(),
        ["fmt", "playground", "typos"],
        "a command outside the pre-commit hook was read as one of its own"
    );
    assert_eq!(
        commands["playground"].0,
        ["playground/*.{ts,json}", "playground/**/*.{ts,tsx,json}"],
        "the list form of `glob:` did not come out as two patterns"
    );
    assert_eq!(
        commands["playground"].1.as_deref(),
        Some("just playground-lint"),
        "the `run:` after a list-valued glob was lost"
    );
    assert_eq!(
        commands["fmt"].0,
        ["*.rs"],
        "the scalar form of `glob:` did not come out"
    );
    assert!(
        commands["typos"].0.is_empty(),
        "a command with no glob came out carrying its neighbour's"
    );
}

#[test]
fn a_glob_matches_across_directories_only_where_it_says_it_does() {
    let cases: &[(&str, &str, bool)] = &[
        // The gap this reader exists to see.
        ("src/**/*.test.ts", "src/outline.test.ts", true),
        ("src/**/*.test.ts", "src/editor/parserState.test.ts", true),
        ("src/**/*.test.ts", "src/components/Toolbar.test.tsx", false),
        // `**/` has to span zero directories as well as many.
        ("src/**/*.ts", "src/outline.ts", true),
        ("src/**/*.ts", "src/styles/theme-urls.ts", true),
        ("src/**/*.ts", "src/App.tsx", false),
        // A single `*` stops at a separator, so a top-level pattern does not
        // quietly cover the tree below it.
        ("*.ts", "vite.config.ts", true),
        ("*.ts", "src/outline.ts", false),
        // A literal, and the `.` in it as a literal too.
        ("package.json", "package.json", true),
        ("package.json", "packageXjson", false),
        // The brace form, which the glob lists here are one edit away from.
        ("src/**/*.{ts,tsx}", "src/components/Toolbar.tsx", true),
        ("src/**/*.{ts,tsx}", "src/styles/shell.css", false),
    ];
    for &(pattern, path, expected) in cases {
        assert_eq!(
            glob_to_regex(pattern).is_match(path),
            expected,
            "`{pattern}` against `{path}` should be {expected}"
        );
    }
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

/// The Justfile as it stands for the pins below: one gate holding the dry run,
/// which is what makes `cargo publish` a command a gate owns.
const PACKAGING_GATE: &str = concat!(
    "[group('gate')]\n",
    "package:\n",
    "    {{_dev}} cargo publish --workspace --dry-run --locked --allow-dirty\n",
);

#[test]
fn the_upload_is_excused_from_the_build_rule_and_a_dry_run_written_out_is_not() {
    // The exemption exists because one command name is two commands, and it is
    // worth exactly as much as its narrowness. `cargo publish` without
    // `--dry-run` is an upload: no recipe may do it, so the workflow spells it
    // and the rule has to let it. `cargo publish` WITH `--dry-run` is the
    // build `just package` owns, and it is the command DEV-224 moved out of
    // this workflow — so an exemption that took the name and not the flags
    // would hand the moved command a standing excuse to move back.
    assert!(
        uploads_to_a_registry(
            "        run: cargo publish --workspace --locked",
            "cargo",
            "publish"
        ),
        "the live upload was not recognised, so the rule will demand a recipe that pushes to \
         crates.io"
    );
    assert!(
        !uploads_to_a_registry(
            "        run: cargo publish --workspace --dry-run --locked",
            "cargo",
            "publish"
        ),
        "the dry run read as an upload; the exemption now covers the very command it was \
         written to keep out of a workflow"
    );
    // The flag is the command's, not the line's.
    assert!(
        uploads_to_a_registry(
            "        run: echo --dry-run && cargo publish --workspace --locked",
            "cargo",
            "publish"
        ),
        "a `--dry-run` sitting before the command was read as belonging to it"
    );
    assert!(
        !uploads_to_a_registry("        run: cargo build --locked", "cargo", "build"),
        "a command that is not a publish was excused as one"
    );

    // And end to end, on the three shapes this file can hold. The pre-DEV-224
    // preflight is the middle one: a build, written out, that no gate defined.
    let owned = commands_owned_by_gates(PACKAGING_GATE);
    let verdicts = |run: &str| {
        workflow_builds(
            "publish-crates.yml",
            &format!("jobs:\n  publish:\n    steps:\n      - run: {run}\n"),
            &owned,
        )
    };

    let upload = verdicts("cargo publish --workspace --locked");
    assert!(
        upload.re_spelled.is_empty() && upload.ungated.is_empty(),
        "the upload was reported: {:?} {:?}",
        upload.re_spelled,
        upload.ungated
    );

    let before = verdicts("cargo publish --workspace --dry-run --locked");
    assert_eq!(
        before.re_spelled.len(),
        1,
        "the dry run this PR moved into a recipe is not reported when a workflow writes it out \
         again: {:?}",
        before.re_spelled
    );

    // The same file with no packaging gate behind it — the state of this repo
    // before DEV-224 — and the same command has to be reported for the other
    // reason: nothing else builds it.
    let ungated = workflow_builds(
        "publish-crates.yml",
        "jobs:\n  publish:\n    steps:\n      - run: cargo publish --workspace --dry-run \
         --locked\n",
        &commands_owned_by_gates("[group('lint')]\nfmt:\n    {{_dev}} cargo fmt --all\n"),
    );
    assert_eq!(
        ungated.ungated.len(),
        1,
        "a build no gate defines went unreported, which is the hole DEV-224 closed: {:?}",
        ungated.ungated
    );

    // A step that names the recipe is neither.
    let called = verdicts("just package");
    assert!(
        called.re_spelled.is_empty() && called.ungated.is_empty(),
        "calling the recipe read as writing the build out: {:?} {:?}",
        called.re_spelled,
        called.ungated
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
    // Surface policy for a published library. This crate is `#![no_main]`
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
// the fuzzing budget, and the backstop wrapped around it
// ---------------------------------------------------------------------------
//
// The section above walked `fuzz-quick`'s single line on every run, and asked
// it one question: does it name a triple. It did. The `timeout` at the front of
// the same line was the reason that recipe had never fuzzed anything.
//
// `cargo fuzz run` is two things — it compiles the target, then executes it —
// so a `timeout` around the whole of it is a wall clock over the build as well
// as the search, and the number beside it had been chosen as the search budget
// plus a little: `timeout 90s` around `-max_total_time=60`. An AddressSanitizer
// build of this graph does not fit in the "little" on a cold runner, so
// fuzz.yml's `sweep (quick)` exited 124 on all five runs it ever had and
// libFuzzer never started once. `fuzz-deep` (360/300) and `fuzz-marathon`
// (1000/900) carried the identical shape; CI happened to run only the first
// (#224).
//
// Nothing in this repo could have said so. `just ci` runs `fuzz-build`, which
// is the compile with no run at all, and every other reader of these recipes
// asked about the text after `--target`. Locally the build is warm, so the
// recipe fits its budget and looks right — the defect exists only on a machine
// that has to compile, which is every machine except the author's.
//
// The three below are therefore about the `timeout` and not about cargo-fuzz's
// flags: where it sits relative to the compile, where its number comes from,
// and whether it still does the one job it is there for.

/// The `Justfile`'s commands with `\` continuations folded, so one entry is one
/// thing a shell runs rather than one line a reader sees.
///
/// [`expanded_recipe_lines`] is per physical line, which is the right grain for
/// "what did this line hand its tool" and the wrong one for "what else is
/// inside the command this `timeout` bounds": the timed fuzz recipe spreads a
/// single `bash -c` over four lines, with the backstop on one of them and the
/// compile it must not cover on another.
fn logical_commands(justfile: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let mut pending: Option<(String, String)> = None;
    for (recipe, line) in expanded_recipe_lines(justfile) {
        // A continuation belongs to the recipe the command STARTED in.
        let (owner, joined) = match pending.take() {
            Some((owner, head)) => (owner, format!("{head} {}", line.trim())),
            None => (recipe, line),
        };
        match joined.trim_end().strip_suffix('\\') {
            Some(head) => pending = Some((owner, head.trim_end().to_owned())),
            None => out.push((owner, joined)),
        }
    }
    out.extend(pending);
    out
}

/// Flags a `cargo fuzz` call takes a detached value after, so that the word
/// behind one is not the target name. Short of parsing cargo-fuzz's clap
/// definition this is a list, and it is the conservative direction: a flag
/// missing from it makes its value read as a target name, which fails the
/// assertion below rather than passing it quietly.
const FUZZ_FLAGS_TAKING_A_VALUE: &[&str] = &[
    "--target",
    "--target-dir",
    "--sanitizer",
    "-s",
    "--jobs",
    "-j",
    "--features",
];

/// The fuzz target a `cargo fuzz <sub>` call names, given tokens starting at
/// the sub-command.
///
/// `None` is a call that names none. For `build` that is every registered
/// target — the widest compile there is, so it answers "was this one built
/// first" with yes rather than escaping the question.
fn fuzz_target_argument(tokens: &[String]) -> Option<String> {
    // Only cargo-fuzz's own side of a bare `--`; past it the words belong to
    // libFuzzer, which has no target argument and plenty of bare ones.
    let own = tokens.split(|token| token == "--").next().unwrap_or(&[]);
    let mut at = 1;
    while let Some(token) = own.get(at) {
        if FUZZ_FLAGS_TAKING_A_VALUE.contains(&token.as_str()) {
            at += 2;
        } else if token.starts_with('-') {
            at += 1;
        } else {
            return Some(token.clone());
        }
    }
    None
}

/// Every `cargo fuzz` call in `tokens` that compiles, as (sub-command, target).
/// [`fuzz_builds`] asks this of one line and keeps the first; a command holds
/// several, and which side of the backstop each falls on is the whole question.
fn fuzz_compiles(tokens: &[String]) -> Vec<(String, Option<String>)> {
    let mut out = Vec::new();
    let mut rest = tokens;
    while let Some(at) = fuzz_build_at(rest) {
        out.push((rest[at].clone(), fuzz_target_argument(&rest[at..])));
        rest = &rest[at + 1..];
    }
    out
}

/// The `timeout` invocation in a command, as written: from the word `timeout`
/// up to the command whose wall clock it bounds.
///
/// Text rather than tokens, because the duration is an arithmetic expansion and
/// [`shell_tokens`] takes `$((` and `))` out as punctuation — which is the
/// right thing for reading a command and destroys the only part of this one
/// that has to be evaluated rather than read.
fn timeout_invocation(command: &str) -> Option<&str> {
    // A word, the way the token readers see one: `--timeout 30` holds this
    // text and is a different flag on a different program.
    let (at, _) = command
        .match_indices("timeout ")
        .find(|&(at, _)| at == 0 || command[..at].ends_with(char::is_whitespace))?;
    let bounded = command[at..].find("cargo")?;
    Some(&command[at..at + bounded])
}

/// Every command in the `Justfile` that wraps a `cargo fuzz` compile in a
/// `timeout`, as (recipe, whole command).
///
/// All of them and not the first. One recipe carries this shape today because
/// this PR folded three copies into it; before that there were three, all three
/// were wrong, and the one CI ran is the one anybody looked at. A reader that
/// stopped at the first would be that arrangement again.
fn timed_fuzz_commands(justfile: &str) -> Vec<(String, String)> {
    logical_commands(justfile)
        .into_iter()
        .filter(|(_, command)| {
            let tokens = shell_tokens(command);
            tokens
                .iter()
                .position(|token| token == "timeout")
                .is_some_and(|at| !fuzz_compiles(&tokens[at..]).is_empty())
        })
        .collect()
}

/// What `timeout` is actually handed when the recipe's `SECONDS` parameter is
/// `seconds`: its flags and its duration, with the arithmetic evaluated.
///
/// Asked of a shell rather than parsed, because a shell is what evaluates it.
/// `timeout` becomes `echo`, so the same text that runs in the recipe is the
/// text that reports here — a second copy computed in Rust would agree with the
/// recipe exactly until the day it mattered.
fn timeout_arguments(invocation: &str, parameter: &str, seconds: i64) -> Vec<String> {
    let script = invocation_for(invocation, parameter, seconds).replacen("timeout", "echo", 1);
    let out = Command::new("bash")
        .args(["-c", &script])
        .output()
        .unwrap_or_else(|e| {
            panic!(
                "running bash: {e}\n\
                 This suite runs inside the dev image (ADR-0002), where bash is installed."
            )
        });
    assert!(
        out.status.success(),
        "the recipe's own `timeout` arguments are not something a shell can evaluate:\n  \
         {script}\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .split_whitespace()
        .map(str::to_owned)
        .collect()
}

/// The name of a recipe parameter as a body spells it: `RANGE="a..b"` is
/// `RANGE` where it is used, and `*ARGS` is `ARGS`.
fn parameter_name(parameter: &str) -> &str {
    let bare = parameter.trim_start_matches(['*', '+', '$']);
    bare.split('=').next().unwrap_or(bare)
}

/// The seconds `timeout` will wait, read off the duration it was handed — its
/// last argument, and the only one with no leading dash.
fn timeout_budget(arguments: &[String]) -> i64 {
    let duration = arguments
        .last()
        .unwrap_or_else(|| panic!("the recipe's `timeout` was handed no arguments at all"));
    duration.trim_end_matches('s').parse().unwrap_or_else(|e| {
        panic!(
            "`timeout {duration}` is not a whole number of seconds ({e}). It may well be a \
             duration `timeout` accepts — `2m`, `1h` — but then the arithmetic this reader \
             compares against libFuzzer's own budget is in different units from it."
        )
    })
}

#[test]
fn every_timed_fuzz_run_has_its_compile_paid_for_outside_the_backstop() {
    // The defect itself, stated over every command in the file rather than over
    // the three recipes that had it. Written that way for the reason the triple
    // assertions above are: a fourth timed recipe is this hole reopened, and a
    // list of today's three would not see it.
    let justfile = read("Justfile");
    let mut bounded = 0usize;
    for (recipe, command) in logical_commands(&justfile) {
        let tokens = shell_tokens(&command);
        let Some(backstop) = tokens.iter().position(|token| token == "timeout") else {
            continue;
        };
        let ahead = fuzz_compiles(&tokens[..backstop]);
        for (sub, target) in fuzz_compiles(&tokens[backstop..]) {
            bounded += 1;
            let paid = ahead.iter().any(|(earlier, built)| {
                earlier == "build" && (built.is_none() || built.as_deref() == target.as_deref())
            });
            assert!(
                paid,
                "`just {recipe}` runs `cargo fuzz {sub}` under a `timeout` and compiles nothing \
                 before it. `cargo fuzz {sub}` builds the target and then executes it, so that \
                 wall clock buys an AddressSanitizer build of the whole graph first and a search \
                 with whatever is left over — on a cold runner, nothing, and an exit 124 that \
                 reads like a hang. Run `cargo fuzz build` for the same target ahead of the \
                 `timeout` and let the backstop bound the run alone (#224):\n      {}",
                command.trim()
            );
        }
    }
    assert!(
        bounded >= 1,
        "no `cargo fuzz` call in the Justfile runs under a `timeout` any more, so this reader is \
         holding nothing. The backstop is not decoration: `-max_total_time` promises an exit only \
         to a libFuzzer that reaches the end of its loop, and a run wedged on one input reaches \
         nothing — which is the failure the sweep's own job-level `timeout-minutes` would then be \
         the first thing to notice, an hour later and with no artifact."
    );
}

/// A `timeout` in the `Justfile` that bounds a fuzz run: the recipe it is in,
/// the invocation as written, and the `-max_total_time` the run inside it hands
/// libFuzzer.
struct TimedFuzzRun {
    recipe: String,
    invocation: String,
    budget: String,
}

/// Every one of them, in file order.
fn timed_fuzz_runs(justfile: &str) -> Vec<TimedFuzzRun> {
    let mut out = Vec::new();
    for (recipe, command) in timed_fuzz_commands(justfile) {
        let invocation = timeout_invocation(&command)
            .unwrap_or_else(|| {
                panic!(
                    "`just {recipe}` has a `timeout` and nothing to read between it and the \
                     command it bounds"
                )
            })
            .to_owned();
        let Some((_, after)) = command.split_once("-max_total_time=") else {
            panic!(
                "`just {recipe}` runs a fuzz target under a `timeout` and gives libFuzzer no \
                 `-max_total_time`. Then the backstop is the only thing that ends the run, and \
                 what ends it is a SIGTERM: no corpus written back, no final stats, exit 124 on a \
                 run that found nothing wrong."
            );
        };
        let budget = after
            .split_whitespace()
            .next()
            .unwrap_or_default()
            .trim_end_matches('\'')
            .to_owned();
        out.push(TimedFuzzRun {
            recipe,
            invocation,
            budget,
        });
    }
    out
}

/// Every recipe that runs `just <recipe>`, with the words it passes after the
/// name. Read off the folded commands, so a call spread over a continuation is
/// still one call.
fn callers_of(justfile: &str, recipe: &str) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for (caller, command) in logical_commands(justfile) {
        if caller == recipe || !line_runs_recipe(&command, recipe) {
            continue;
        }
        let arguments: Vec<String> = dotted_words(strip_comment(&command))
            .skip_while(|word| *word != recipe)
            .skip(1)
            .map(str::to_owned)
            .collect();
        out.push((caller, arguments));
    }
    out
}

/// The recipe parameter `run`'s duration is written in terms of, and every
/// search that backstop is therefore asked to bound, as (who asked, seconds).
///
/// A literal duration bounds exactly one search, the one written beside it. A
/// parameterised one bounds whatever its callers pass, and the callers are then
/// the only place in the file those numbers appear at all — which is why this
/// question is asked of them rather than of the recipe.
fn searches_bounded_by(justfile: &str, run: &TimedFuzzRun) -> (String, Vec<(String, i64)>) {
    let spelled: BTreeSet<&str> = words(&run.invocation).collect();
    let parameters = recipe_parameters(recipe_header(justfile, &run.recipe));
    let Some(at) = parameters
        .iter()
        .position(|p| spelled.contains(parameter_name(p)))
    else {
        let seconds: i64 = run.budget.trim_end_matches('s').parse().unwrap_or_else(|e| {
            panic!(
                "`just {}` gives libFuzzer `-max_total_time={}`, which is not a whole number of \
                 seconds ({e})",
                run.recipe, run.budget
            )
        });
        return (String::new(), vec![(run.recipe.clone(), seconds)]);
    };
    let parameter = parameter_name(parameters[at]);
    let callers = callers_of(justfile, &run.recipe);
    assert!(
        !callers.is_empty(),
        "`just {}` takes a fuzzing budget as a parameter and nothing calls it, so no number in \
         this file is the budget it computes its backstop from",
        run.recipe
    );
    let mut out = Vec::new();
    for (caller, arguments) in callers {
        let passed = arguments.get(at).unwrap_or_else(|| {
            panic!(
                "`just {caller}` calls `{}` with {arguments:?} and gives it no `{parameter}`. Its \
                 budget is then the empty string, its backstop is whatever `$(( + grace ))` comes \
                 to, and neither is what the recipe's own doc comment promises.",
                run.recipe
            )
        });
        let seconds: i64 = passed.parse().unwrap_or_else(|e| {
            panic!(
                "`just {caller}` passes `{passed}` as `{}`'s `{parameter}` ({e}). The arguments \
                 are positional and the other one is a target name, so the two swapped round is a \
                 recipe that fuzzes a target called `{passed}` for as many seconds as the target \
                 name comes to — which in shell arithmetic is zero.",
                run.recipe
            )
        });
        out.push((caller, seconds));
    }
    (parameter.to_owned(), out)
}

#[test]
fn the_backstop_counts_a_wall_clock_derived_from_the_budget_it_wraps() {
    // The other half of the defect, and the half that would have grown back.
    // `timeout 90s` beside `-max_total_time=60` is two numbers written by hand
    // that have to agree, in three recipes, each carrying its own pair — and
    // the pair was wrong in all three. Correcting them is a fix for today; the
    // arithmetic is what stops the fourth pair from being written wrong.
    let justfile = read("Justfile");
    let runs = timed_fuzz_runs(&justfile);
    assert!(
        !runs.is_empty(),
        "no Justfile command wraps a `cargo fuzz` call in a `timeout`; the reader is not finding \
         it, or the backstop is gone"
    );

    for run in &runs {
        let spelled: BTreeSet<&str> = words(&run.invocation).collect();
        for word in words(&run.budget) {
            assert!(
                spelled.contains(word),
                "`just {}` bounds a `-max_total_time={}` run with `{}`, and the two numbers have \
                 nothing to do with each other. They are one number: the backstop is the search \
                 budget plus a shutdown, so it has to be computed from it. Written side by side \
                 they drift — which they had, in all three recipes, in the direction that spent \
                 the whole search on the build (#224).",
                run.recipe,
                run.budget,
                run.invocation.trim()
            );
        }
    }
}

#[test]
fn the_backstop_outlasts_every_search_it_is_asked_to_bound() {
    // Naming the budget is only half of "computed from it": `timeout $SECONDS`
    // around `-max_total_time=$SECONDS` names it and truncates it. Measured
    // against the numbers the callers actually pass, so what is held is the
    // recipe as it is invoked rather than as it is parameterised — and a
    // caller that passes the two positional arguments the wrong way round is
    // read here rather than at the far end of a sweep.
    let justfile = read("Justfile");
    let runs = timed_fuzz_runs(&justfile);
    assert!(
        !runs.is_empty(),
        "no Justfile command wraps a `cargo fuzz` call in a `timeout`, so this reader is holding \
         nothing"
    );

    for run in &runs {
        let (parameter, searches) = searches_bounded_by(&justfile, run);
        for (asked_by, seconds) in &searches {
            let wall = timeout_budget(&timeout_arguments(&run.invocation, &parameter, *seconds));
            assert!(
                wall > *seconds,
                "`just {asked_by}` asks for a {seconds}-second search and the backstop around it \
                 fires at {wall}s. A backstop that cuts the search short is not a backstop; it is \
                 a shorter search that reports itself as a hang — and libFuzzer killed mid-loop \
                 writes back neither the corpus it grew nor the stats that say what it covered."
            );
        }
    }
}

#[test]
fn the_backstop_around_a_fuzz_run_kills_a_run_that_never_ends() {
    // Run rather than read, and the reason is the change that made the two
    // assertions above pass: the duration stopped being the literal `90s` and
    // became `$(( … ))`, i.e. text that is correct only if a shell evaluates it
    // where the recipe puts it. Quoted one layer differently and `timeout`
    // receives the expression verbatim, exits 125 without waiting, and every
    // fuzz run in this repo loses its backstop while both readers above go on
    // reporting it present. The same goes for `--kill-after`, which neither of
    // them reads at all.
    let justfile = read("Justfile");
    let runs = timed_fuzz_runs(&justfile);
    assert!(
        !runs.is_empty(),
        "no Justfile command wraps a `cargo fuzz` call in a `timeout`, so nothing here is \
         demonstrated"
    );

    for run in &runs {
        let recipe = &run.recipe;
        let (parameter, _) = searches_bounded_by(&justfile, run);

        // The grace this recipe allows on top of a search, measured by asking
        // the expression what it comes to at zero. A budget of one second is
        // then a search of `1 - grace`, which is nonsense as a search and is
        // exactly the point: what is timed here is the backstop, not a fuzzer.
        let grace = timeout_budget(&timeout_arguments(&run.invocation, &parameter, 0));
        let shrunk = 1 - grace;
        let wall = timeout_budget(&timeout_arguments(&run.invocation, &parameter, shrunk));
        assert_eq!(
            wall, 1,
            "`just {recipe}` computes a {wall}-second backstop for a {shrunk}-second search, so \
             its wall clock is not the search budget plus a fixed grace and this demonstration \
             cannot shrink the recipe's own invocation to something it can wait for. Either the \
             duration went back to a literal — the arrangement the reader above rejects — or it \
             grew a factor, in which case rewrite the shrink and not the recipe."
        );
        let ready = invocation_for(&run.invocation, &parameter, shrunk);

        // The demonstration: the recipe's own invocation, around something that
        // never ends.
        let started = Instant::now();
        let hung = bash(&format!("{ready} sleep 300"));
        let waited = started.elapsed();
        assert_eq!(
            hung.code(),
            Some(124),
            "`just {recipe}`'s backstop did not kill a command that never ends. 124 is the status \
             `timeout` reports when it fires, and the status fuzz.yml reported five times for a \
             reason that was never a hang."
        );
        assert!(
            waited < Duration::from_secs(30),
            "`just {recipe}`'s backstop fired after {waited:?} on a one-second budget, so what \
             ended the command was not the duration the recipe computes"
        );

        // The control. A backstop that fires on a run which finished is a red X
        // on every sweep, and a red X on every sweep is what taught six pull
        // requests to merge past this job in the first place.
        let finished = bash(&format!("{ready} true"));
        assert!(
            finished.success(),
            "`just {recipe}`'s backstop reports {finished:?} for a command that exited immediately"
        );
    }
}

/// The recipe's `timeout` invocation with its budget parameter filled in, ready
/// for a shell.
fn invocation_for(invocation: &str, parameter: &str, seconds: i64) -> String {
    invocation.replace(&format!("{{{{{parameter}}}}}"), &seconds.to_string())
}

/// Run a script the way the recipes run theirs, and report only how it ended.
fn bash(script: &str) -> process::ExitStatus {
    Command::new("bash")
        .args(["-c", script])
        .status()
        .unwrap_or_else(|e| panic!("running bash: {e}"))
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
const FUZZ_RECIPES_THAT_ENUMERATE_NOTHING: &[(&str, &str)] = &[
    (
        "fuzz-build",
        "`cargo fuzz build` with no target argument compiles every `[[bin]]` the fuzz \
         manifest declares. That enumeration is cargo-fuzz's own and is one step closer \
         to the registry than `cargo fuzz list` is, so reading the list here would be a \
         copy rather than a cure",
    ),
    (
        "fuzz-lock",
        "it acts on the fuzz workspace's lockfile and not on its targets: one \
         `cargo update` re-resolves the single graph every `[[bin]]` is built \
         from, so there is no per-target step for a list to drive. Recorded here \
         rather than filtered out of `whole_set_fuzz_recipes`, and that carve-out is \
         a real cost — the classifier's premise, that a recipe taking no argument \
         acts on every registered target, is false for this one. Narrowing the \
         premise is worse: any rule that let this recipe out (\"names no target \
         directory\", \"never reaches cargo-fuzz\") would also let out a sweep that \
         globbed `fuzz/corpus/*` instead of asking `cargo fuzz list`, which is the \
         hand-written listing the whole check exists to keep from growing back",
    ),
];

/// Fuzz recipes that spell a target's name themselves, each with the reason.
/// Every entry is a LIVE DEFECT rather than an exemption: the name is a
/// hand-maintained copy of one line of `cargo fuzz list`, and the whole point
/// of the derivation is that there are none of those.
const FUZZ_RECIPES_THAT_NAME_A_TARGET: &[(&str, &str)] = &[(
    "fuzz-seed",
    "the input FORMAT a target reads is a per-target fact with nowhere else in this \
     repo to live, and two targets read something other than \"the whole input is \
     UTF-8 source\". `sjis_decode` hands its bytes to `decode_sjis`, which rejects a \
     UTF-8 seed at its first multi-byte character, so its seeds are transcoded to \
     CP932; `options_space` reads a two-byte option mask before its source, so its \
     seeds carry that prefix. That is the defect, not the workaround, and it is one \
     this entry predicted in as many words before `options_space` existed: a second \
     `[[bin]]` with an input format of its own was seeded with documents it could \
     not read, `just fuzz-seed` reported its seed count as cheerfully as for the \
     rest, and nothing here said so. Nothing HERE still does — this text grew a \
     clause. What now does is `fuzz_regressions.rs`, which reads every target's \
     corpus back through the shape that target reads and fails when the bytes are \
     not the documents they were made from; the name in the recipe stays a copy, \
     and a copy that lies is now caught one directory over rather than never",
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
    //
    // `some target's corpus, verbatim` is as far as this file can put it, and
    // it is not far enough: only one target is seeded with the document as it
    // stands, so the `any` below is held up by `parse_render` alone and says
    // nothing about the other four. It cannot say more from here — a seed is
    // in the shape ITS target reads, and the decoders for those shapes live in
    // the library's own test suite, one of them in a crate this one does not
    // depend on. `fuzz_regressions.rs` asks the same question per target with
    // those decoders in hand; what stays here is the half that needs git.
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
    // committed lockfile instead. Committed is the whole of it: a file a clone
    // does not get is re-resolved from scratch on the next build, and every
    // question the repo asks about the fuzz graph — `just fuzz-build`'s "did
    // this build rewrite it", `just verify-version-pins`' "do the two lockfiles
    // agree" — is then asked of whatever that resolution happened to produce.
    // `fuzz/.gitignore` listed `Cargo.lock` until DEV-293, so this is a state
    // the repo has really been in, with every message in those gates still
    // perfectly accurate.
    //
    // "Would a clone get it" rather than "is it committed": the ignore rule is
    // what silently un-ships a file, and it is the half a person does not
    // look at. On the branch that first adds the file it is carried and still
    // untracked, which this cannot tell apart — a gap that closes at the first
    // commit and cannot reopen.
    let carried = git_carried(FUZZ_LOCK);
    assert!(
        carried.contains(FUZZ_LOCK),
        "git would not carry {FUZZ_LOCK} to the next clone. It is the only thing binding the \
         fuzz workspace's resolution — cargo-fuzz takes no `--locked` — so without it every \
         build resolves its own graph and the gates over that file compare a fresh resolution \
         against itself."
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

/// The ADR every other one is copied from, and so the file that spells the
/// status vocabulary.
const ADR_TEMPLATE: &str = "docs/adr/0000-template.md";

/// The `- Status:` a document declares, lower-cased. `None` outside `docs/adr`.
fn adr_status(text: &str) -> Option<String> {
    text.lines()
        .find_map(|line| line.trim().strip_prefix("- Status:"))
        .map(|status| status.trim().to_lowercase())
}

/// Is this document in force, as opposed to recording something that was?
///
/// An ADR is when it is `accepted`: that status means the decision is in
/// force, which is the authority that makes a wrong sentence in one expensive
/// — see the section on the check ADR-0015 said could not run. Anything else
/// with a status, and the records `crates/xtask/src/main.rs` already names,
/// record rather than instruct.
///
/// The statuses come off [`ADR_TEMPLATE`]'s placeholder line rather than a
/// list here, because a status this reader did not recognise used to read as
/// not-in-force in silence — taking the whole document out of every rule
/// below with every gate still green.
fn describes_this_repo_now(label: &str, text: &str) -> bool {
    let Some(status) = adr_status(text) else {
        return !history_paths()
            .iter()
            .any(|history| label == history || label.starts_with(&format!("{history}/")));
    };
    // `superseded by ADR-XXXX` is offered with a number the writer supplies,
    // so an offer is matched on the literal half of itself.
    let placeholder = adr_status(&read(ADR_TEMPLATE))
        .unwrap_or_else(|| panic!("`{ADR_TEMPLATE}` declares no `- Status:` line to read"));
    let offered: Vec<&str> = placeholder
        .trim_matches(|ch| ch == '{' || ch == '}')
        .split('|')
        .map(|offer| offer.split("adr-").next().unwrap_or_default().trim())
        .collect();
    assert!(
        label == ADR_TEMPLATE || offered.iter().any(|offer| status.starts_with(offer)),
        "{label} declares `- Status: {status}` and `{ADR_TEMPLATE}` offers {offered:?}. A status \
         nothing recognises reads as not-in-force, so a typo takes the document out of every rule \
         below and fails nothing."
    );
    status == "accepted"
}

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

/// Words that hand the sentence to a new subject, which is what makes a comma
/// the end of a clause rather than an interruption in one.
const CONJUNCTIONS: &[&str] = &[
    "and", "but", "or", "yet", "though", "although", "whereas", "while",
];

/// The clauses of one unwrapped paragraph, outside backticks so a command
/// keeps its own punctuation.
///
/// A comma cuts only when a conjunction follows it. Cutting at every comma
/// was the boundary a negation's reach could not cross, and an interrupted
/// negation — "does not, on any pull request, run `just test`" — landed on the
/// far side of one. What the cut is really for is CONTRIBUTING.md's two native
/// gates, which "need a toolchain the dev image has not got, AND they run the
/// same recipe there": the conjunction is the word doing that work, so it is
/// the word asked for.
fn clauses_of(paragraph: &str) -> Vec<&str> {
    let characters: Vec<(usize, char)> = paragraph.char_indices().collect();
    let mut out = Vec::new();
    let mut code = false;
    let mut start = 0;
    for (at, &(offset, ch)) in characters.iter().enumerate() {
        code ^= ch == '`';
        // A full stop ends a sentence unless a word runs on through it, so
        // `crates.io`, `docs.yml` and `0.4.1` stay in one piece.
        let stop = matches!(ch, '.' | '!' | '?')
            && !characters
                .get(at + 1)
                .is_some_and(|&(_, next)| next.is_ascii_alphanumeric());
        let hands_over = ch == ','
            && paragraph[offset + ch.len_utf8()..]
                .split_whitespace()
                .next()
                .is_some_and(|word| CONJUNCTIONS.contains(&word.to_lowercase().as_str()));
        if !code && (stop || hands_over || matches!(ch, ';' | ':' | '(' | ')' | '—' | '–')) {
            out.push(paragraph[start..offset].trim());
            start = offset + ch.len_utf8();
        }
    }
    out.push(paragraph[start..].trim());
    out.retain(|clause| !clause.is_empty());
    out
}

/// Every clause of `text` in which a word from `negations` reaches a word from
/// `nouns`, with the line its paragraph starts at.
///
/// Paragraphs, because where a sentence wraps is a typesetting decision: a
/// line-at-a-time reader could attribute a denial to the tool it was about
/// only when both landed on one physical line, so reflowing defeated it.
/// [`clauses_of`] then bounds the reach. Fenced blocks are dropped: a
/// transcript is not a claim.
fn denials_in(text: &str, negations: &[&str], nouns: &[&str]) -> Vec<(usize, String)> {
    let mut paragraphs: Vec<(usize, String)> = Vec::new();
    let (mut fenced, mut open) = (false, false);
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        let fence = line.starts_with("```");
        fenced ^= fence;
        if line.is_empty() || fence || fenced {
            open = false;
        } else if open {
            let (_, paragraph) = paragraphs.last_mut().expect("open implies a paragraph");
            paragraph.push(' ');
            paragraph.push_str(line);
        } else {
            paragraphs.push((index + 1, line.to_owned()));
            open = true;
        }
    }

    let mut out = Vec::new();
    for (line, paragraph) in &paragraphs {
        for clause in clauses_of(paragraph) {
            let words = prose_words(clause);
            let denies = words.iter().enumerate().any(|(at, word)| {
                negations.contains(&word.as_str())
                    && words
                        .iter()
                        .skip(at + 1)
                        .take(NEGATION_REACH)
                        .any(|later| nouns.contains(&later.as_str()))
            });
            if denies {
                out.push((*line, clause.to_owned()));
            }
        }
    }
    out
}

/// Every `.md` in force that describes this repository to a reader: the prose
/// at the root, everything under `docs/` and `.github/`, and each workspace
/// member's own page.
///
/// Walked rather than listed, for the reason the conformance-figure reader is
/// walked: the defect these rules exist for is a copy nobody knew about. The
/// three listed roots this replaced missed all five crate READMEs — the pages
/// a reader meets first, published inside the packages themselves — and
/// `.github/`, whose pull-request template addresses a contributor at the
/// moment the claims in it are being acted on.
///
/// A directory predicate rather than every tracked `.md`, because the rest are
/// content: `playground/examples/` and the EPUB sample manuscript are input
/// this suite renders, and a denial written into one of those is a sentence
/// nobody is being told.
fn ci_prose_files() -> Vec<(String, String)> {
    let members: BTreeSet<String> = workspace_members()
        .iter()
        .map(|member| crate_dir(&member.path).to_owned())
        .collect();
    let out: Vec<(String, String)> = git_tracked(&[])
        .into_iter()
        .filter(|label| {
            let at = label.rsplit_once('/').map_or("", |(head, _)| head);
            let addresses_a_reader = |root: &str| at == root || at.starts_with(&format!("{root}/"));
            Path::new(label)
                .extension()
                .is_some_and(|kind| kind == "md")
                && (at.is_empty()
                    || addresses_a_reader("docs")
                    || addresses_a_reader(".github")
                    || members.contains(at))
        })
        .map(|label| {
            let text = read(&label);
            (label, text)
        })
        .filter(|(label, text)| describes_this_repo_now(label, text))
        .collect();
    // Asked here rather than in each of the three rules that read this, since
    // a reader that has stopped finding the documents passes all of them at
    // once. An accepted ADR is the class whose excusal as history let
    // ADR-0015's own sentence stand.
    assert!(
        out.iter().any(|(label, _)| label == "SECURITY.md")
            && out.iter().any(|(label, _)| label.starts_with("docs/adr/")),
        "the prose reader found {:?}",
        out.iter().map(|(label, _)| label).collect::<Vec<_>>()
    );
    out
}

#[test]
fn no_document_denies_a_scheduled_run_this_repo_makes() {
    let documents = ci_prose_files();
    let mut denials = Vec::new();
    for (label, text) in &documents {
        for (line, denial) in denials_in(text, NEGATIONS, SCHEDULE_NOUNS) {
            denials.push(format!("  {label}:{line}: {denial}"));
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
}

// The other direction of this rule used to require SECURITY.md to contain the
// literal string `.github/workflows/audit.yml`. That is a gate compelling a
// document to restate a config file, which is the defect above with the sign
// flipped; the sentence it compelled has gone with it. SECURITY.md keeps only
// why the schedule is not redundant with the pull request, which no file says.

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

    // And the other half of the prose: every entry ends with a sentence about
    // when to drop it, and issue #197 is the story of one of those sentences
    // being false from the day it was written — `libfuzzer-sys`' entry said to
    // drop it once the fuzz crate moved to libafl_libfuzzer, which turns out
    // to depend on libfuzzer-sys itself, so the condition could never come
    // due. Four months, nothing able to say so, and the sentence read as a
    // plan every time somebody opened the file.
    //
    // What IS decidable here is the precondition every one of those sentences
    // shares: the exemption waives a package this repo actually resolves. Two
    // lockfiles, because the fuzz crate is its own workspace and its
    // dependencies appear in neither the other file nor `cargo deny` — which
    // is exactly why libfuzzer-sys needs an entry here in the first place. An
    // exemption for a package no lockfile mentions waives nothing, and waits
    // to waive whatever takes that name next.
    let lockfiles = [
        ("Cargo.lock", read("Cargo.lock")),
        (FUZZ_LOCK, read(FUZZ_LOCK)),
    ];
    for (name, text) in &lockfiles {
        assert!(
            text.contains("[[package]]"),
            "{name} parsed as {} byte(s) with no `[[package]]` table; the reader is not finding \
             the lockfile and the check below would excuse every exemption",
            text.len()
        );
    }
    for package in &exempted {
        let declaration = format!("name = \"{package}\"");
        assert!(
            lockfiles
                .iter()
                .any(|(_, text)| text.contains(&declaration)),
            "{LICENCE_REVIEW} exempts `{package}` from the licence check and neither {} nor \
             {FUZZ_LOCK} resolves it any more. The entry now waives a name rather than a \
             dependency: delete it, and its row in {LICENCE_WATCH} with it. This is the \
             `Drop this entry once …` sentence beside it, evaluated — the half of those \
             sentences that does not need somebody to remember.",
            lockfiles[0].0
        );
    }
}

/// The reporter a scheduled job calls when it has none of its own: a local
/// composite action that files ONE rolling issue per title.
const LOCAL_REPORTER: &str = "./.github/actions/report-failure";

/// Where that action is written.
const LOCAL_REPORTER_FILE: &str = ".github/actions/report-failure/action.yml";

/// A step that gets a finding off the runner and in front of somebody, and
/// what it takes for one to be able to.
///
/// A channel is a STEP, which is the second half of this rule and was missing
/// from the first. `issues: write` on a job used to be read as "this job files
/// an issue", and that is a claim about what the job COULD do: three jobs hold
/// that grant now for the sake of one step each, so deleting any one of those
/// steps leaves the grant behind, and a rule that stopped at the grant would
/// go on reporting a channel that is gone.
struct Filer {
    /// What a step's `uses:` names. Matched inside the reference, so the
    /// commit a pin carries does not have to be restated here.
    uses: &'static str,
    /// The write permission it cannot file with, where it needs one.
    /// `upload-artifact` needs none — an artifact belongs to the run that
    /// wrote it.
    grant: Option<&'static str>,
    /// What it does, in the words this rule reports.
    does: &'static str,
    /// Does it report a failure an EARLIER step produced?
    ///
    /// GitHub applies an implicit `success()` to any `if:` that names no
    /// status function, so a step that reports somebody else's failure and
    /// does not say `failure()` is skipped on precisely the run it exists for.
    /// The two scanners below are not in that position: each one IS the check,
    /// and files what it found on its way to failing.
    after_an_earlier_failure: bool,
}

/// Everything this repo has that can put a finding in front of a human. A way
/// of reporting that is not here reads as no channel at all — which fails
/// loudly and is added to in one line, the direction this rule would rather
/// be wrong in.
const FILERS: &[Filer] = &[
    Filer {
        uses: LOCAL_REPORTER,
        grant: Some("issues"),
        does: "opens a rolling issue",
        after_an_earlier_failure: true,
    },
    Filer {
        uses: ADVISORY_REPORTER,
        grant: Some("issues"),
        does: "opens an issue per advisory",
        after_an_earlier_failure: false,
    },
    Filer {
        uses: "github/codeql-action/analyze",
        grant: Some("security-events"),
        does: "raises a code-scanning alert",
        after_an_earlier_failure: false,
    },
    Filer {
        uses: "actions/upload-artifact",
        grant: None,
        does: "uploads the evidence",
        after_an_earlier_failure: true,
    },
];

/// What one step's `uses:` names, in either shape a step is written in.
/// `None` for every other line, which is what keeps a `uses:` discussed in a
/// comment or quoted in a `body:` out of the answer.
fn step_uses(line: &str) -> Option<&str> {
    let body = strip_comment(line).trim();
    let body = body.strip_prefix("- ").unwrap_or(body);
    body.strip_prefix("uses:").map(str::trim)
}

/// Can this step run on a run where an earlier step already failed? Only a
/// status function makes it so: an `if:` without one, and an absent `if:`,
/// both mean `success()`.
fn runs_after_a_failure(step: &[&str]) -> bool {
    step.iter()
        .filter_map(|line| strip_comment(line).trim().strip_prefix("if:"))
        .any(|expression| {
            ["failure()", "always()", "!cancelled()"]
                .iter()
                .any(|reaches| expression.contains(reaches))
        })
}

/// The `permissions:` block that governs one job.
///
/// A job's own replaces the workflow's outright rather than adding to it, and
/// a job without one inherits the workflow's whole. Reading the grant off the
/// job's own lines alone would get the second case right by accident and the
/// first wrong.
fn job_permissions<'a>(workflow: &'a str, lines: &[&'a str]) -> Vec<&'a str> {
    if declares_key(lines, "permissions") {
        nested_block(lines, "permissions")
    } else {
        top_level_block(workflow, "permissions")
    }
}

/// Is `permission` granted for writing by this block?
fn grants_write(permissions: &[&str], permission: &str) -> bool {
    let wanted = format!("{permission}: write");
    permissions
        .iter()
        .any(|line| strip_comment(line).trim() == wanted)
}

/// Every step of a job that files something, paired with the [`Filer`] it
/// files by. Every one of them, in the order they run: which is the whole
/// difference between this and the `position` it replaced.
fn filer_steps(lines: &[&str]) -> Vec<(usize, &'static Filer)> {
    lines
        .iter()
        .enumerate()
        .filter_map(|(at, line)| {
            let reference = step_uses(line)?;
            let filer = FILERS.iter().find(|filer| reference.contains(filer.uses))?;
            Some((at, filer))
        })
        .collect()
}

/// What one reporter's `if:` narrows it to, when that is a comparison against
/// a step output: the output read, whether the test is `==`, and the literal
/// it is read against.
///
/// A condition on the EVENT is deliberately not one of these. Every reporter
/// here carries `github.event_name == 'schedule'`, and the rule below is only
/// ever asked about the scheduled run — so that half is this rule's own scope
/// rather than a gap in the job's coverage.
fn output_test(step: &[&str]) -> Option<(String, bool, String)> {
    let compare =
        Regex::new(r"(steps\.[A-Za-z0-9_-]+\.outputs\.[A-Za-z0-9_-]+)\s*(==|!=)\s*'([^']*)'")
            .expect("the comparison pattern is a literal");
    step.iter()
        .filter_map(|line| strip_comment(line).trim().strip_prefix("if:"))
        .find_map(|expression| {
            compare.captures(expression).map(|caught| {
                (
                    caught[1].to_owned(),
                    &caught[2] == "==",
                    caught[3].to_owned(),
                )
            })
        })
}

/// Do a job's reporters, between them, answer for every way that job can fail?
///
/// Asked only of a job [`reporting_channel`] has already said yes about, so
/// every step read here is one that can run on a failing run. What is left is
/// the second question a job with more than one reporter raises and a job with
/// one cannot: a reporter behind `steps.<id>.outputs.<name> == 'x'` answers
/// for SOME failures, and the rest of them reach whoever is watching the other
/// conditions.
///
/// audit.yml's `dependabot` job is the first here to split that way, and the
/// split is the point of it — "the posture is off" and "the posture could not
/// be read" are different findings for different people, and filing the second
/// under the first is the false alarm that teaches a reader to stop opening the
/// channel. A pair like that is a channel only while it is a PARTITION.
/// Conditions that both narrow leave whatever neither names reaching nobody;
/// conditions that overlap file two issues for one run, which is the duplicate
/// the rolling reporter was built to prevent.
fn every_failure_reaches_a_channel(lines: &[&str]) -> Result<(), String> {
    let mut narrowed: BTreeMap<String, Vec<(bool, String)>> = BTreeMap::new();
    let mut answers_for_everything = false;
    for (at, _) in filer_steps(lines) {
        match output_test(&step_around(lines, at)) {
            None => answers_for_everything = true,
            Some((output, equals, literal)) => {
                narrowed.entry(output).or_default().push((equals, literal));
            }
        }
    }
    if answers_for_everything || narrowed.is_empty() {
        return Ok(());
    }

    for (output, tests) in &narrowed {
        let mut seen = BTreeSet::new();
        for (equals, literal) in tests {
            if !seen.insert((equals, literal)) {
                let operator = if *equals { "==" } else { "!=" };
                return Err(format!(
                    "puts two of its reporters behind the same `{output} {operator} '{literal}'`, \
                     so one failure opens two issues — the duplicate a rolling reporter exists to \
                     prevent, and the surest way to teach a reader to stop reading the channel"
                ));
            }
        }
    }
    let complementary = narrowed.values().any(|tests| {
        tests.iter().any(|(equals, literal)| {
            *equals
                && tests
                    .iter()
                    .any(|(other, value)| !other && value == literal)
        })
    });
    if complementary {
        return Ok(());
    }
    let listed: Vec<String> = narrowed
        .iter()
        .flat_map(|(output, tests)| {
            tests.iter().map(move |(equals, literal)| {
                let operator = if *equals { "==" } else { "!=" };
                format!("`{output} {operator} '{literal}'`")
            })
        })
        .collect();
    Err(format!(
        "narrows every one of its reporters to a step output ({}) and no two of those are \
         complements, so a failure matching none of them is filed by neither",
        listed.join(", ")
    ))
}

/// How ONE JOB of a workflow gets its finding off the runner and in front of
/// somebody, or what it is missing.
///
/// Per job, because the workflow was the wrong unit and said so: audit.yml
/// passed this rule on the strength of its `report` job while its `licences`
/// job — the watch over the dependency-review waivers — failed into exactly
/// the red square the rule forbids, one level below where the rule could see
/// it. A channel does not transfer between jobs. They are separate runners
/// with separate tokens, `report` files what `rustsec/audit-check` finds and
/// nothing else, and a job that goes red ends wherever its own `permissions:`
/// and its own steps leave it.
fn reporting_channel(workflow: &str, job: &str) -> Result<&'static str, String> {
    let Some(lines) = job_lines(workflow, job) else {
        return Err(format!("has no `{job}:` job to read a channel off"));
    };
    let permissions = job_permissions(workflow, &lines);
    let mut missing = Vec::new();
    let mut works = None;
    let mut calls_one = false;
    // EVERY filing step of the job, rather than the first one per filer that
    // answers. `position` was right for as long as a job had one reporter, and
    // wrong from the moment audit.yml's `dependabot` job arrived with two: the
    // second sat entirely outside this net, so the finding it exists for — a
    // posture that could not be READ, as against one read and wrong — could
    // stop being filed with this rule still green on the strength of the
    // reporter above it. A job with two channels has to keep both.
    for (at, filer) in filer_steps(&lines) {
        calls_one = true;
        if let Some(needed) = filer.grant
            && !grants_write(&permissions, needed)
        {
            missing.push(format!(
                "calls `{}` without the `{needed}: write` it files with",
                filer.uses
            ));
            continue;
        }
        if !filer.after_an_earlier_failure || runs_after_a_failure(&step_around(&lines, at)) {
            works = works.or(Some(filer.does));
            continue;
        }
        missing.push(format!(
            "calls `{}` behind an `if:` that names no status function, so GitHub applies the \
             implicit `success()` and skips it on exactly the run it reports",
            filer.uses
        ));
    }
    if let Some(does) = works
        && missing.is_empty()
    {
        return Ok(does);
    }
    // Read off FILERS rather than listed again, and only where the job calls
    // nothing at all: a grant held beside a step that IS called has already
    // been reported on above, by the sentence about that step.
    if !calls_one {
        let held: BTreeSet<&str> = FILERS
            .iter()
            .filter_map(|filer| filer.grant)
            .filter(|permission| grants_write(&permissions, permission))
            .collect();
        for permission in held {
            missing.push(format!(
                "holds `{permission}: write` and calls nothing that uses it — a permission is \
                 what a channel would need, not a channel"
            ));
        }
    }
    Err(if missing.is_empty() {
        "neither files an issue, raises a code-scanning alert nor uploads an artifact".to_owned()
    } else {
        missing.join("; and ")
    })
}

/// The scheduled jobs whose failure reaches nobody, one sentence each.
///
/// Split out from the rule below so the rule can be asked about a tree other
/// than this one. A rule only ever run against the tree it passes on is a rule
/// nobody has watched say no, and that is the half of #200's acceptance the
/// mutation tests underneath cover.
fn jobs_with_no_channel(workflows: &[(String, String)]) -> Vec<String> {
    let mut silent = Vec::new();
    for (label, text) in workflows.iter().filter(|(_, text)| runs_on_a_cron(text)) {
        // The floor that matters once the unit is a job, and derived rather
        // than counted: a workflow defines at least one job or it runs
        // nothing, so finding none in one is the reader having stopped reading
        // that file — the way a per-job rule passes vacuously.
        let jobs = job_keys(text);
        assert!(
            !jobs.is_empty(),
            "{label} runs on a cron and no job was read out of it. The jobs reader is not finding \
             them, so this workflow answers this rule with nothing."
        );
        for job in jobs {
            if let Err(why) = reporting_channel(text, &job) {
                silent.push(format!(
                    "  {label}: `{job}` runs on a cron, blocks no merge, and {why}. A red square \
                     on a workflow nobody is required to read is not a control, and a sibling \
                     job's reporter is not this job's channel."
                ));
                continue;
            }
            // The job has a channel. What is left is whether that channel
            // answers for every way the job can fail — the question a job with
            // one reporter never raised and a job that splits its reporting
            // over two conditions raises immediately.
            let lines = job_lines(text, &job).expect("the channel was read off this job's lines");
            if let Err(gap) = every_failure_reaches_a_channel(&lines) {
                silent.push(format!(
                    "  {label}: `{job}` runs on a cron, blocks no merge, and {gap}. A failure \
                     that matches none of a job's reporters reaches nobody by exactly the route a \
                     job with no reporter at all does."
                ));
            }
        }
    }
    silent
}

#[test]
fn a_scheduled_run_that_blocks_nothing_says_where_its_failure_goes() {
    // The generalisation of the fuzz workflow's upload step and of audit.yml's
    // `report` job. Neither is a required check — a fuzzer finding is a bug
    // report, an advisory is somebody else's publication — so nothing about
    // either failing stops a merge or lands in a pull request. Whatever
    // reaches a human has to be arranged by the workflow itself.
    //
    // Every JOB of one, which is the half this rule used to miss. A reporter
    // answers for what it ran: audit.yml's `report` files RustSec advisories,
    // and that made the whole file pass while its `licences` job — the watch
    // over the dependency-review waivers — and its cargo-deny leg both failed
    // into the red square this test is about.
    //
    // And a channel is a STEP that files, which is the half a per-job reading
    // does not fix by itself. `issues: write` was the whole of the evidence
    // once; three jobs hold that grant now for the sake of one step each, so a
    // reading that stopped at the permission would have gone on reporting a
    // channel for as long as the grant outlived the step that used it.
    //
    // There is no carve-out list any more. The one entry it carried,
    // release-pins.yml, files its own issue now, and the workflow that
    // outgrew the per-workflow reading files two. An exception belongs back
    // here as a named entry with the reason beside it — not as an empty shape
    // waiting for one.
    let workflows = read_workflows();
    let scheduled = workflows
        .iter()
        .filter(|(_, text)| runs_on_a_cron(text))
        .count();
    assert!(
        scheduled >= 3,
        "only {scheduled} workflow(s) read as running on a cron; the trigger reader has stopped \
         finding them and everything below passes vacuously"
    );

    let silent = jobs_with_no_channel(&workflows);
    assert!(
        silent.is_empty(),
        "scheduled jobs whose findings reach nobody:\n{}",
        silent.join("\n")
    );
}

/// `text` with the `key:` block of `job` taken out, located by the readers the
/// rule itself uses rather than by a line number that drifts.
fn without_a_block(text: &str, job: &str, key: &str) -> String {
    let lines = job_lines(text, job).unwrap_or_else(|| panic!("no `{job}:` job to mutate"));
    let header = format!("{key}:");
    let opens = lines
        .iter()
        .find(|line| strip_comment(line).trim() == header)
        .unwrap_or_else(|| panic!("`{job}` declares no `{key}:` to take out"));
    let mut block = vec![*opens];
    block.extend(nested_block(&lines, key));
    let cut = block.join("\n");
    assert!(
        text.contains(&cut),
        "the `{key}:` block of `{job}` is not the run of consecutive lines this mutation assumed"
    );
    text.replacen(&cut, "", 1)
}

/// `text` with the step of `job` that `uses:` `reference` taken out.
fn without_a_step(text: &str, job: &str, reference: &str) -> String {
    let lines = job_lines(text, job).unwrap_or_else(|| panic!("no `{job}:` job to mutate"));
    let at = lines
        .iter()
        .position(|line| step_uses(line).is_some_and(|uses| uses.contains(reference)))
        .unwrap_or_else(|| panic!("`{job}` has no step using `{reference}` to take out"));
    let cut = step_around(&lines, at).join("\n");
    assert!(
        text.contains(&cut),
        "the `{reference}` step of `{job}` is not the run of consecutive lines this mutation \
         assumed"
    );
    text.replacen(&cut, "", 1)
}

/// One workflow, in the shape [`jobs_with_no_channel`] reads.
fn one_workflow(label: &str, text: String) -> Vec<(String, String)> {
    vec![(label.to_owned(), text)]
}

/// Is `job` among the jobs these sentences are about?
fn named(silent: &[String], job: &str) -> bool {
    silent
        .iter()
        .any(|sentence| sentence.contains(&format!("`{job}`")))
}

/// The other scheduled workflow these mutations are cut from. The nightly one
/// is [`LICENCE_WATCH`] — the same file under the name the licence rule reads
/// it by, spelled once rather than twice.
const WEEKLY_PINS: &str = ".github/workflows/release-pins.yml";

#[test]
fn the_rule_names_the_job_the_per_workflow_reading_let_through() {
    // #200's third acceptance criterion, run rather than reasoned: the tightened
    // rule has to be watched saying NO to the tree it was tightened for, and a
    // rule that has only ever been asked about a tree it passes on is a rule
    // whose reader could have stopped reading.
    //
    // The mutation IS that tree: audit.yml with the channel taken back off its
    // `licences` job, which is what #198 shipped and what the per-workflow
    // reading called fine on the strength of `report` two jobs above.
    let text = read(LICENCE_WATCH);
    let before = without_a_step(&text, "licences", LOCAL_REPORTER);
    let before = without_a_block(&before, "licences", "permissions");
    assert_ne!(
        before, text,
        "the mutation changed nothing, so it proves nothing"
    );
    assert!(
        runs_on_a_cron(&before),
        "the mutated workflow no longer reads as scheduled, so this rule would skip it for a \
         reason that has nothing to do with what is being measured"
    );
    assert_eq!(
        reporting_channel(&before, "report").as_deref(),
        Ok("opens an issue per advisory"),
        "the mutated workflow has to keep the sibling reporter that made the old reading pass — \
         without it this test would be measuring a workflow with no channel anywhere"
    );

    let silent = jobs_with_no_channel(&one_workflow(LICENCE_WATCH, before));
    assert!(
        named(&silent, "licences"),
        "a cron job with no channel of its own went unreported while a sibling job held one. That \
         is the per-workflow reading this rule replaced:\n{silent:?}"
    );
    assert!(
        !named(&silent, "report"),
        "the job that does report was reported too, so the rule is not reading per job at all:\n\
         {silent:?}"
    );
}

#[test]
fn a_permission_no_step_uses_is_not_a_channel() {
    // The half a per-job rule still misses if a channel is read off
    // `permissions:`. Three jobs hold `issues: write` for one step each; delete
    // the step and the grant stays behind, describing a channel that is gone.
    let text = read(LICENCE_WATCH);
    let stripped = without_a_step(&text, "licences", LOCAL_REPORTER);
    let lines = job_lines(&stripped, "licences").expect("the mutation kept the job");
    assert!(
        grants_write(&job_permissions(&stripped, &lines), "issues"),
        "the mutation removed the grant as well as the step, so it cannot show what reading the \
         grant alone would have concluded"
    );

    let silent = jobs_with_no_channel(&one_workflow(LICENCE_WATCH, stripped));
    assert!(
        named(&silent, "licences"),
        "a job holding `issues: write` with nothing in it that files an issue was read as having \
         a channel:\n{silent:?}"
    );
}

#[test]
fn a_reporter_that_cannot_run_after_the_failure_is_not_a_channel() {
    // The other way the wiring dies quietly. `if: ${{ failure() && … }}` is
    // what puts the reporter on the failing run; drop the status function and
    // GitHub supplies `success()`, so the step is skipped exactly when there is
    // something to report — with the permission granted, the action called, the
    // workflow green in review and the finding still in the Actions tab.
    let text = read(WEEKLY_PINS);
    assert_eq!(
        reporting_channel(&text, "freshness").as_deref(),
        Ok("opens a rolling issue"),
        "the tree this mutation starts from does not have the channel it is about to break"
    );

    let unguarded = text.replace("failure() && ", "");
    assert_ne!(
        unguarded, text,
        "no `failure() && …` guard was found to drop, so the mutation proves nothing"
    );
    let silent = jobs_with_no_channel(&one_workflow(WEEKLY_PINS, unguarded));
    assert!(
        named(&silent, "freshness"),
        "a reporting step that cannot run on a failing run was read as a channel:\n{silent:?}"
    );
}

#[test]
fn a_channel_is_read_off_the_job_that_would_need_it() {
    // The reader, against the shapes that decide what a grant belongs to.
    // Every one of them was answered wrong by the scan this replaced, which
    // asked whether the WORKFLOW TEXT contained `issues: write` anywhere.
    let workflow = concat!(
        "name: probe\n",
        "\n",
        "on:\n",
        "  schedule:\n",
        "    - cron: \"40 15 * * *\"\n",
        "\n",
        "permissions:\n",
        "  contents: read\n",
        "\n",
        "jobs:\n",
        "  report:\n",
        "    permissions:\n",
        "      contents: read\n",
        "      issues: write\n",
        "    steps:\n",
        "      - uses: rustsec/audit-check@0123456789abcdef0123456789abcdef01234567\n",
        "  licences:\n",
        "    steps:\n",
        "      - name: Compare each exemption against what crates.io now declares\n",
        "        run: ./check\n",
    );
    assert_eq!(
        reporting_channel(workflow, "report").as_deref(),
        Ok("opens an issue per advisory"),
        "the job that has a channel was not read as having one, so nothing below means anything"
    );
    assert!(
        reporting_channel(workflow, "licences").is_err(),
        "a sibling job's grant was read as this job's channel"
    );

    // A workflow-level grant reaches a job that declares no `permissions:` of
    // its own, and is REPLACED — not extended — by one that does.
    let reporting = concat!(
        "        run: ./check\n",
        "      - uses: ./.github/actions/report-failure\n",
        "        if: ${{ failure() }}\n",
    );
    let inheriting = workflow
        .replace(
            "permissions:\n  contents: read\n",
            "permissions:\n  issues: write\n",
        )
        .replace("        run: ./check\n", reporting);
    assert_eq!(
        reporting_channel(&inheriting, "licences").as_deref(),
        Ok("opens a rolling issue"),
        "a workflow-level grant did not reach the job that declares no `permissions:` of its own"
    );
    let replacing = inheriting.replace(
        "  licences:\n    steps:\n",
        "  licences:\n    permissions:\n      contents: read\n    steps:\n",
    );
    assert!(
        reporting_channel(&replacing, "licences").is_err(),
        "a job's own `permissions:` replaces the workflow's outright, so `contents: read` there \
         means the workflow's `issues: write` is gone — it was read as still in force"
    );

    // A step's `with:` input is indented like a permission and is not one, and
    // an upload in a sibling job is that job's evidence and not this one's.
    let misread = workflow.replace(
        "        run: ./check\n",
        &format!("{reporting}        with:\n          issues: write\n"),
    );
    assert!(
        reporting_channel(&misread, "licences")
            .is_err_and(|why| why.contains("without the `issues: write` it files with")),
        "a `with:` input spelled like a permission was counted as the grant"
    );
    let elsewhere = workflow.replace(
        "      - uses: rustsec/audit-check@0123456789abcdef0123456789abcdef01234567\n",
        concat!(
            "      - uses: rustsec/audit-check@0123456789abcdef0123456789abcdef01234567\n",
            "      - if: failure()\n",
            "        uses: actions/upload-artifact@0123456789abcdef0123456789abcdef01234567\n",
        ),
    );
    assert!(
        reporting_channel(&elsewhere, "licences").is_err(),
        "an upload step in a sibling job was read as this job's evidence"
    );
}

/// The job that made the reading above insufficient: the first here to report
/// through two channels rather than one.
const TWO_CHANNEL_JOB: &str = "dependabot";

/// What narrows its second reporter, as written. The FIRST reporter is the
/// complement of this, so a mutation that names this string names exactly one
/// of the two.
const SECOND_REPORTER: &str = "steps.posture.outputs.posture != 'off'";

#[test]
fn a_second_reporter_that_cannot_run_is_not_covered_by_the_first() {
    // The hole the fourth job of audit.yml opened in the rule above it. That
    // rule asked each `Filer` for the FIRST step using it and stopped there, so
    // a job with two reporters was judged on one: break the second and the
    // first still answers, the workflow still reads as having a channel, and
    // the finding the second exists for — a posture that could not be read,
    // as against read and wrong — silently stops being filed.
    //
    // The failure mode is the one the test above this section already
    // measures, one level down: `if: ${{ failure() && … }}` is what puts a
    // reporter on the failing run, and GitHub supplies `success()` for an
    // expression that names no status function.
    let text = read(LICENCE_WATCH);
    assert_eq!(
        reporting_channel(&text, TWO_CHANNEL_JOB).as_deref(),
        Ok("opens a rolling issue"),
        "the tree this mutation starts from does not have the channel it is about to break"
    );
    let lines = job_lines(&text, TWO_CHANNEL_JOB).expect("the job the channel was just read off");
    assert_eq!(
        filer_steps(&lines).len(),
        2,
        "`{TWO_CHANNEL_JOB}` is the job with two reporters this test is about. Either it stopped \
         splitting its reporting or the step reader stopped finding the steps; either way what \
         follows measures nothing."
    );

    let guarded = format!("failure() && github.event_name == 'schedule' && {SECOND_REPORTER}");
    let unguarded = guarded
        .strip_prefix("failure() && ")
        .expect("the guard is the prefix of the string just built");
    let broken = text.replace(&guarded, unguarded);
    assert_ne!(
        broken, text,
        "no second reporter guarded by `failure() && …` was found to break, so this proves nothing"
    );
    assert!(
        reporting_channel(&broken, TWO_CHANNEL_JOB)
            .is_err_and(|why| why.contains("implicit `success()`")),
        "a job whose second reporter cannot run on a failing run was read as having its channel, \
         on the strength of the first: {:?}",
        reporting_channel(&broken, TWO_CHANNEL_JOB)
    );

    let silent = jobs_with_no_channel(&one_workflow(LICENCE_WATCH, broken));
    assert!(
        named(&silent, TWO_CHANNEL_JOB),
        "the rule did not name the job whose second channel is gone:\n{silent:?}"
    );
}

#[test]
fn reporters_that_all_narrow_leave_whatever_neither_names_unreported() {
    // The other half of splitting one job's reporting in two, and the one no
    // reading of the STEPS can see: both steps are present, both can run on a
    // failing run, and between them they still answer for only some of the
    // ways the job fails. `== 'off'` and `!= 'off'` partition every run;
    // `== 'off'` and `== 'blind'` leave the third answer reaching nobody, and
    // `== 'off'` twice files two issues for one failure.
    let text = read(LICENCE_WATCH);
    let lines = job_lines(&text, TWO_CHANNEL_JOB).expect("the two-channel job");
    assert_eq!(
        every_failure_reaches_a_channel(&lines),
        Ok(()),
        "the tree this mutation starts from is already uncovered, so nothing below is about the \
         mutation"
    );

    for (mutation, shape) in [
        (
            "steps.posture.outputs.posture == 'blind'",
            "a gap neither reporter names",
        ),
        (
            "steps.posture.outputs.posture == 'off'",
            "an overlap both reporters name",
        ),
    ] {
        let mutated = text.replace(SECOND_REPORTER, mutation);
        assert_ne!(
            mutated, text,
            "the second reporter's condition was not found, so {shape} was never introduced"
        );
        assert!(
            reporting_channel(&mutated, TWO_CHANNEL_JOB).is_ok(),
            "the mutation broke the reporting STEPS as well as their conditions, so it cannot \
             show what the coverage half of the rule sees on its own"
        );
        let silent = jobs_with_no_channel(&one_workflow(LICENCE_WATCH, mutated));
        assert!(
            named(&silent, TWO_CHANNEL_JOB),
            "{shape} went unreported — two reporting steps were counted as reporting, without \
             anything asking what they report on:\n{silent:?}"
        );
    }
}

/// The body of a `key: |` block scalar, dedented to column zero.
fn block_scalar(text: &str, key: &str) -> Option<String> {
    let opens = |line: &str| {
        let body = line.trim();
        body == format!("{key}: |") || body == format!("{key}: |-")
    };
    let indent = |line: &str| line.len() - line.trim_start().len();
    let lines: Vec<&str> = text.lines().collect();
    let at = lines.iter().position(|line| opens(line))?;
    let open_indent = indent(lines[at]);
    let body: Vec<&str> = lines[at + 1..]
        .iter()
        .take_while(|line| line.trim().is_empty() || indent(line) > open_indent)
        .copied()
        .collect();
    let margin = body
        .iter()
        .filter(|line| !line.trim().is_empty())
        .map(|line| indent(line))
        .min()?;
    Some(
        body.iter()
            .map(|line| line.get(margin..).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n"),
    )
}

/// The one shell script the local reporter runs.
///
/// Run rather than read, because nothing else in this repo reads it at all:
/// `just actionlint` discovers `.github/workflows/` and not `.github/actions/`,
/// and it is invoked with `-shellcheck=` off on purpose. The dedup this script
/// implements is the whole of #199's "repeated runs do not duplicate", and
/// until here that promise was a sentence in a comment.
fn reporter_script() -> String {
    let action = read(LOCAL_REPORTER_FILE);
    let script = block_scalar(&action, "run").unwrap_or_else(|| {
        panic!(
            "no `run: |` block in {LOCAL_REPORTER_FILE}; the reporter is not a shell step any more"
        )
    });
    assert!(
        script.contains("gh issue create"),
        "the script read out of {LOCAL_REPORTER_FILE} files nothing, so every case below would \
         pass on an empty string"
    );
    script
}

/// A `gh` that answers out of files instead of out of GitHub.
///
/// A shell function rather than a program on `PATH`: bash resolves a function
/// before it searches `PATH`, so what runs underneath it is the action's own
/// text with nothing altered and no executable bit to set. `cat` gives the
/// same SIGPIPE behaviour a real `gh` would, which is the point of the third
/// case below.
const GH_STUB: &str = r#"
gh() {
  printf '%s\n' "${*:-}" >> "$STUB_CALLS"
  case "${1:-} ${2:-}" in
    'issue list') cat "$STUB_TITLES" ;;
    'issue create')
      shift 2
      while [ "$#" -gt 0 ]; do
        case "$1" in
          --title) printf '%s\n' "$2" >> "$STUB_FILED"; shift 2 ;;
          --body-file) cat "$2" >> "$STUB_BODY"; shift 2 ;;
          *) shift ;;
        esac
      done
      ;;
  esac
}
"#;

const REPORTED_RUN: &str = "https://github.invalid/P4suta/aozora-flavored-markdown/actions/runs/42";
const REPORTED_BODY: &str = "A pin in dist-workspace.toml is behind its upstream.";

/// What one run of the reporter did.
struct Reported {
    ok: bool,
    stderr: String,
    /// The title of each issue it created.
    filed: Vec<String>,
    body: String,
    /// Every `gh` command line it ran.
    calls: Vec<String>,
}

/// Run the reporter against an issue list of `open_titles`, reporting `title`.
fn run_the_reporter(open_titles: &[String], title: &str) -> Reported {
    let dir = scratch("report-failure");
    let write = |name: &str, content: String| {
        let path = dir.join(name);
        fs::write(&path, content).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
        path
    };
    let titles = write("open-titles", open_titles.join("\n") + "\n");
    let calls = write("calls", String::new());
    let filed = write("filed", String::new());
    let body = write("body", String::new());

    let out = Command::new("bash")
        .arg("-c")
        .arg(format!("{GH_STUB}\n{}", reporter_script()))
        .env("STUB_TITLES", &titles)
        .env("STUB_CALLS", &calls)
        .env("STUB_FILED", &filed)
        .env("STUB_BODY", &body)
        .env("RUNNER_TEMP", &dir)
        .env("GH_TOKEN", "probe")
        .env("GH_REPO", "P4suta/aozora-flavored-markdown")
        .env("TITLE", title)
        .env("BODY", REPORTED_BODY)
        .env("RUN_URL", REPORTED_RUN)
        .output()
        .unwrap_or_else(|e| panic!("running the reporter: {e}"));

    let lines = |path: &Path| {
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(str::to_owned)
            .collect::<Vec<String>>()
    };
    let reported = Reported {
        ok: out.status.success(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        filed: lines(&filed),
        body: fs::read_to_string(&body).unwrap_or_default(),
        calls: lines(&calls),
    };
    drop(fs::remove_dir_all(&dir));
    reported
}

#[test]
fn the_rolling_reporter_files_once_and_stays_quiet_while_that_issue_is_open() {
    let title = "audit: a licence exemption no longer holds";

    let first = run_the_reporter(&[], title);
    assert!(
        first.ok,
        "the reporter exited non-zero against an empty issue list:\n{}",
        first.stderr
    );
    assert!(
        first
            .calls
            .iter()
            .any(|call| call.starts_with("issue list")),
        "nothing asked which issues are open, so what ran is not the script in \
         {LOCAL_REPORTER_FILE}"
    );
    assert_eq!(
        first.filed,
        [title],
        "the first failing run filed {:?} rather than one issue titled {title:?}",
        first.filed
    );
    assert!(
        first.body.contains(REPORTED_RUN),
        "the filed issue does not name the run that produced it, so the reader has the finding \
         and no way to see it:\n{}",
        first.body
    );

    let again = run_the_reporter(&[title.to_owned()], title);
    assert!(again.ok, "the reporter exited non-zero:\n{}", again.stderr);
    assert!(
        again.filed.is_empty(),
        "a repeat failure filed {:?} while an issue of that title was already open. Drift recurs \
         every run until somebody fixes it, and a weekly copy of a report that is already open is \
         how a channel stops being read (#199).",
        again.filed
    );

    // The hazard the script's own comment names, measured. `gh … | grep -q`
    // leaves gh writing into a pipe grep has closed, and under `pipefail` that
    // reads back as "no such issue" — which files the duplicate above on every
    // run whose issue list outgrows a pipe buffer.
    let crowded: Vec<String> = iter::once(title.to_owned())
        .chain((0..5_000).map(|n| format!("some unrelated open issue, number {n}")))
        .collect();
    let under_load = run_the_reporter(&crowded, title);
    assert!(
        under_load.ok,
        "the reporter died reading a long issue list:\n{}",
        under_load.stderr
    );
    assert!(
        under_load.filed.is_empty(),
        "an issue list too long for a pipe buffer made the reporter file {:?}, which is the \
         duplicate it read the whole list to avoid",
        under_load.filed
    );
}

#[test]
fn the_rolling_reporters_key_is_the_whole_title_read_literally() {
    let title = "release-pins: a hand-maintained release pin has frozen behind upstream";
    let wider = run_the_reporter(&[format!("{title} on the macOS runner")], title);
    assert_eq!(
        wider.filed,
        [title],
        "an open issue whose title merely CONTAINS this one suppressed the report. Without a \
         whole-line match every longer title is a wildcard over the shorter ones, and the report \
         nobody filed is the one nobody reads."
    );

    let dotted = "release-pins: dist v1.0 is behind";
    let alike = run_the_reporter(&["release-pins: dist v1X0 is behind".to_owned()], dotted);
    assert_eq!(
        alike.filed,
        [dotted],
        "the title was matched as a regular expression, so an open issue that merely looks alike \
         swallowed the report"
    );
}

/// The `with:` input `key` of one step, unquoted.
fn step_input(step: &[&str], key: &str) -> Option<String> {
    let header = format!("{key}:");
    nested_block(step, "with").iter().find_map(|line| {
        strip_comment(line)
            .trim()
            .strip_prefix(header.as_str())
            .map(|value| value.trim().trim_matches('"').to_owned())
    })
}

#[test]
fn every_rolling_report_is_keyed_on_a_title_that_cannot_change_between_runs() {
    // What makes the reporter roll is that a repeat failure computes the SAME
    // key. Two ways that stops being true, neither of which changes a line of
    // the action: a title carrying `${{ github.run_id }}` files one issue per
    // run, and two jobs sharing a title file one issue between them — where
    // whichever fails first suppresses the other for as long as it stays open.
    let workflows = read_workflows();
    let mut keys: Vec<(String, String)> = Vec::new();
    for (label, text) in &workflows {
        for job in job_keys(text) {
            let lines = job_lines(text, &job).unwrap_or_default();
            for (at, line) in lines.iter().enumerate() {
                if step_uses(line) != Some(LOCAL_REPORTER) {
                    continue;
                }
                let step = step_around(&lines, at);
                let site = format!("{label}: `{job}`");
                let title = step_input(&step, "title").unwrap_or_else(|| {
                    panic!(
                        "{site} calls the reporter with no `title:`, which is the key it files \
                            under"
                    )
                });
                assert!(
                    step_input(&step, "body").is_some(),
                    "{site} calls the reporter with no `body:`, so the issue it files says only \
                     that something failed"
                );
                keys.push((site, title));
            }
        }
    }
    // The floor on what was found, derived rather than written down. The walk
    // above reads the calls job by job; the same texts hold the same `uses:`
    // lines whether or not the job reader finds the jobs around them, so the
    // two readings disagree exactly when the walk has stopped seeing part of
    // the tree — which is how every assertion below starts passing vacuously.
    let called: usize = workflows
        .iter()
        .flat_map(|(_, text)| text.lines())
        .filter(|line| step_uses(line) == Some(LOCAL_REPORTER))
        .count();
    assert!(
        called > 0,
        "no step in {} workflow(s) calls `{LOCAL_REPORTER}` any more, so this rule is about \
         nothing",
        workflows.len()
    );
    assert_eq!(
        keys.len(),
        called,
        "{called} step(s) call `{LOCAL_REPORTER}` and the walk over jobs found {}; the job or step \
         reader has stopped seeing part of the tree, and what it missed is what this rule would \
         otherwise have judged.",
        keys.len()
    );

    for (site, title) in &keys {
        assert!(
            !title.contains("${{"),
            "{site} keys its rolling issue on `{title}`, which is computed per run. A key that \
             changes with the run is a new issue every run, which is the duplicate flood the \
             rolling design exists to avoid."
        );
        assert!(
            !title.trim().is_empty(),
            "{site} files under an empty title, so every report matches every other"
        );
    }

    let distinct: BTreeSet<&String> = keys.iter().map(|(_, title)| title).collect();
    assert_eq!(
        distinct.len(),
        keys.len(),
        "two scheduled jobs file under one title, so whichever fails first suppresses the other's \
         report for as long as its issue is open:\n{keys:?}"
    );
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

// The rule that used to sit here held two repository settings — Dependabot
// alerts and Dependabot security updates — against a sentence in SECURITY.md
// and against some scheduled job running `gh api repos/<this repo>/<endpoint>`
// for each. Its subject was that sentence and the spelling of a shell command,
// never the settings themselves: `gh api … || true` satisfied it in full. The
// control is audit.yml's `dependabot` job, which asks GitHub what the settings
// actually ARE and reports two ways on the answer; a test asserting that the
// job is written the way it is written was a second copy of the job. Its list
// of endpoints could not be derived either — the mapping from the phrase
// "Dependabot security updates" to `automated-security-fixes` is GitHub's
// vocabulary and appears in no file here — so it could only ever have been
// topped up by hand, which is what a hand-written net is for.

// ---------------------------------------------------------------------------
// the wire between a step and the condition that reads it
// ---------------------------------------------------------------------------

/// The `id:` a step declares, in either shape a step is written in.
fn step_id(line: &str) -> Option<&str> {
    let body = strip_comment(line).trim();
    let body = body.strip_prefix("- ").unwrap_or(body);
    let id = body.strip_prefix("id:")?.trim();
    (!id.is_empty()).then_some(id)
}

/// Every `steps.<id>.outputs.<name>` a workflow reads, with the job reading it.
fn output_references(workflow: &str) -> Vec<(String, String, String)> {
    let reference = Regex::new(r"steps\.([A-Za-z0-9_-]+)\.outputs\.([A-Za-z0-9_-]+)")
        .expect("the reference pattern is a literal");
    let mut out = Vec::new();
    for job in job_keys(workflow) {
        let Some(lines) = job_lines(workflow, &job) else {
            continue;
        };
        for line in lines {
            for caught in reference.captures_iter(strip_comment(line)) {
                out.push((job.clone(), caught[1].to_owned(), caught[2].to_owned()));
            }
        }
    }
    out
}

/// References that name a step the job has not got, or an output the step they
/// name never writes.
fn unresolved_outputs(workflows: &[(String, String)]) -> Vec<String> {
    let mut dangling = Vec::new();
    for (label, text) in workflows {
        for (job, id, name) in output_references(text) {
            let lines = job_lines(text, &job).expect("the reference was read out of this job");
            let Some(at) = lines
                .iter()
                .position(|line| step_id(line) == Some(id.as_str()))
            else {
                dangling.push(format!(
                    "  {label}: `{job}` reads `steps.{id}.outputs.{name}` and holds no step with \
                     `id: {id}`"
                ));
                continue;
            };
            let step = step_around(&lines, at).join("\n");
            // A step that `uses:` an action gets its outputs from that action's
            // own `action.yml`, which is not this repository's to read for a
            // third-party pin. A `run:` step has one way to produce one, and it
            // is in the script.
            if !step.contains("run:") {
                continue;
            }
            let written = step.contains("GITHUB_OUTPUT")
                && (step.contains(&format!("{name}=")) || step.contains(&format!("{name}<<")));
            if !written {
                dangling.push(format!(
                    "  {label}: `{job}` reads `steps.{id}.outputs.{name}` and the script under \
                     `id: {id}` writes no `{name}` to `$GITHUB_OUTPUT`"
                ));
            }
        }
    }
    dangling
}

#[test]
fn every_step_output_a_condition_reads_is_one_the_step_it_names_writes() {
    // The seam the two-channel job runs on, and the quietest one in this
    // repository. An unset `steps.<id>.outputs.<name>` is not an error at run
    // time: it is the empty string. So a renamed `id:`, or a script that stops
    // writing the output, leaves `== 'off'` false for ever and `!= 'off'` true
    // for ever — and the job goes on reporting, through the wrong one of its
    // two channels, on every failure it ever has. Every gate stays green: the
    // steps are all there, the guards all name `failure()`, and the reporters
    // still hold `issues: write`.
    let workflows = read_workflows();
    let references: Vec<(String, String, String)> = workflows
        .iter()
        .flat_map(|(_, text)| output_references(text))
        .collect();
    assert!(
        references.len() >= 8,
        "only {} step-output reference(s) were read out of {} workflow(s); the reader has stopped \
         finding them and this rule passes on nothing",
        references.len(),
        workflows.len()
    );
    assert!(
        references
            .iter()
            .any(|(_, id, name)| id == "posture" && name == "posture"),
        "the reference the two reporting channels of audit.yml split on was not among the {} \
         found",
        references.len()
    );

    let dangling = unresolved_outputs(&workflows);
    assert!(
        dangling.is_empty(),
        "step outputs read by a condition that nothing sets:\n{}\n\
         An unset output is the empty string rather than an error, so every condition resting on \
         one answers the same way for ever and no run says a word about it.",
        dangling.join("\n")
    );
}

#[test]
fn a_renamed_step_and_an_unwritten_output_are_both_read_as_dangling() {
    let text = read(LICENCE_WATCH);
    assert!(
        unresolved_outputs(&one_workflow(LICENCE_WATCH, text.clone())).is_empty(),
        "the workflow these mutations start from already reads as dangling"
    );

    // The `id:` moves and the two conditions reading it do not.
    let renamed = text.replace("id: posture", "id: settings");
    assert_ne!(renamed, text, "no `id: posture` was found to rename");
    let dangling = unresolved_outputs(&one_workflow(LICENCE_WATCH, renamed));
    assert!(
        dangling
            .iter()
            .any(|sentence| sentence.contains("holds no step with `id: posture`")),
        "a condition reading a step that is not in the job any more was read as wired:\n\
         {dangling:?}"
    );

    // The step keeps its name and stops writing what the conditions read.
    let silent = text.replace("printf 'posture=off\\n' >> \"$GITHUB_OUTPUT\"", ":");
    assert_ne!(
        silent, text,
        "no write to `$GITHUB_OUTPUT` was found to cut"
    );
    let dangling = unresolved_outputs(&one_workflow(LICENCE_WATCH, silent));
    assert!(
        dangling
            .iter()
            .any(|sentence| sentence.contains("writes no `posture`")),
        "a condition reading an output no script sets was read as wired:\n{dangling:?}"
    );
}

// ---------------------------------------------------------------------------
// the net a prose gate casts
// ---------------------------------------------------------------------------
//
// The fifth way a gate checks nothing, and the one this repo keeps rebuilding:
// the gate runs, is strict, is asked at the right moment, and reads a list of
// file kinds that is not the list of file kinds the defect appears in.
//
// `comment-discipline` was the case. It existed to catch a retired upstream
// path written in prose, and its file list was `[("rs", "//"), ("toml", "#")]`
// — so `UPSTREAM_DIFF.md` described a vendored tree that no longer existed,
// in full, with every gate green, because no gate in this repo could open a
// `.md` file. Vale replaces it (DEV-221).
//
// A tool swap does not close that by itself. The replacement was first written
// as `git ls-files -- '*.md' '*.rs' '*.toml'`, which is the same hand-written
// list of kinds one language over, and it had the same hole in a different
// place: the first run over every tracked file found a retired crate named in
// the `Justfile`, a file with no extension for a pathspec to match. The rule
// itself had the third version of it — Vale's default scope drops what
// Markdown calls code, and a crate name in a document is written in backticks,
// so the file kind the gate was added for hid the drift in its usual spelling.
//
// So the rules below are about the net rather than the tool: what it is handed,
// what it looks at inside that, whether what it finds can fail, whether an
// exemption is wider than the record it was written for, and whether CI ever
// reaches it on a diff that touches only what it reads.

/// The files `git ls-files` lists for `pathspec`, as repo-relative paths.
///
/// Shelling out to git rather than walking: what this repo authors is what git
/// tracks, and a second definition of that is the thing the gate below exists
/// to keep from being written.
fn git_tracked(pathspec: &[String]) -> BTreeSet<String> {
    let out = Command::new("git")
        .arg("ls-files")
        .arg("-z")
        .args(pathspec)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("running `git ls-files {pathspec:?}`: {e}"));
    assert!(
        out.status.success(),
        "`git ls-files {pathspec:?}` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|path| !path.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The arguments the `vale` recipe hands `git ls-files`, read off the recipe
/// itself.
///
/// Read rather than assumed, because the whole assertion is about what the
/// gate reaches: a pathspec added back here has to show up as files the gate
/// no longer sees, not as a comment nobody compares against anything.
fn prose_gate_pathspec(justfile: &str) -> Vec<String> {
    let body = recipe_body(justfile, "vale").join("\n");
    let at = body
        .find("git ls-files")
        .unwrap_or_else(|| panic!("`just vale` no longer lists its files with `git ls-files`"));
    let rest = &body[at..];
    let end = rest.find(')').unwrap_or(rest.len());
    shell_tokens(&rest[..end])
        .into_iter()
        .skip_while(|token| token != "ls-files")
        .skip(1)
        .filter(|token| token != "-z" && token != "--")
        .collect()
}

#[test]
fn every_file_this_repo_tracks_is_one_the_prose_gate_reads() {
    // The invariant the replaced scan failed, stated about the net instead of
    // about the tool. `SCANNED_FILES` was two kinds; `'*.md' '*.rs' '*.toml'`
    // was three; the answer that does not have to be maintained is "all of
    // them", because a retired name rots identically in a `Justfile` comment,
    // a workflow comment, a completion script and a Markdown paragraph, and
    // nothing about the file's extension makes the sentence more or less true.
    let justfile = read("Justfile");
    assert!(
        recipes_in_group(&justfile, "gate").contains("vale"),
        "`vale` is not in the gate manifest, so nothing below is a gate at all"
    );

    let tracked = git_tracked(&[]);
    assert!(
        tracked.len() > 1_000,
        "`git ls-files` came back with {} files; the reader is not seeing the tree",
        tracked.len()
    );

    let pathspec = prose_gate_pathspec(&justfile);
    let reached = git_tracked(&pathspec);
    let mut missed: Vec<&String> = tracked.difference(&reached).collect();
    missed.sort();
    let shown: Vec<&&String> = missed.iter().take(12).collect();
    assert!(
        missed.is_empty(),
        "`just vale` narrows its file list to {pathspec:?}, which leaves {} tracked file(s) \
         outside every prose gate this repo has, among them {shown:?}.\n\
         That is the defect the gate replaced, rebuilt: a hand-written list of file kinds is \
         always shorter than the list of places a sentence can rot. Hand Vale `git ls-files` \
         with no pathspec.",
        missed.len()
    );
}

/// The banned tokens, read out of the rule file.
///
/// Every probe below draws its needle from here rather than spelling one, for
/// two reasons: this file is itself scanned by the gate, and a rule file
/// emptied of tokens would otherwise leave every probe passing against a rule
/// that bans nothing.
fn retired_tokens() -> Vec<String> {
    let rule = read("styles/Aozora/RetiredPaths.yml");
    let mut out = Vec::new();
    let mut in_tokens = false;
    for line in rule.lines() {
        if !line.starts_with([' ', '\t', '-']) {
            in_tokens = line.trim_end() == "tokens:";
            continue;
        }
        let Some(item) = line.trim().strip_prefix("- ").filter(|_| in_tokens) else {
            continue;
        };
        let token = item.trim();
        let token = quoted_literal(token, '\'')
            .or_else(|| quoted_literal(token, '"'))
            .unwrap_or(token);
        out.push(token.to_owned());
    }
    out
}

/// A banned name written both ways: hyphenated as a manifest spells it,
/// underscored as an intra-doc link spells it.
///
/// The rule writes the pair as one `[-_]` token, and an intra-doc link to a
/// crate that has gone away is where this drift actually accumulates, so a
/// probe that only ever tries one spelling would pass on half a rule.
fn retired_name_in_both_spellings() -> (String, String) {
    let token = retired_tokens()
        .into_iter()
        .find(|token| token.contains("[-_]"))
        .unwrap_or_else(|| {
            panic!(
                "the rule bans no name whose two spellings differ, so nothing here can check \
                 that both are banned"
            )
        });
    (token.replace("[-_]", "-"), token.replace("[-_]", "_"))
}

/// `vale`, or `None` when this image is inside the one-merge window the
/// recipe's `command -v vale || curl` bridge exists for.
///
/// CI runs `just test` inside a published dev image that predates the
/// Dockerfile adding a tool, so the probe below cannot insist on the binary
/// without going red on the merge that installs it. What it can insist on is
/// that an absence is still that window: the recipe knows how to fetch it.
fn vale_or_the_bridge_that_installs_it() -> Option<PathBuf> {
    let path = env::var_os("PATH").unwrap_or_default();
    let found = env::split_paths(&path)
        .map(|dir| dir.join("vale"))
        .find(|candidate| candidate.is_file());
    if found.is_some() {
        return found;
    }
    // That the version the bridge fetches is still pinned where the recipe
    // looks for it is `every_search_this_repo_runs_to_read_a_value_finds_one`'s
    // now: it compiles the recipe's own `grep` against the `Dockerfile`, which
    // the `contains("VALE_VERSION=")` that used to stand here could not.
    assert!(
        recipe_body(&read("Justfile"), "vale")
            .join("\n")
            .contains("command -v vale"),
        "vale is not in this image and the recipe carries no way to get it"
    );
    None
}

/// A directory holding this repo's real Vale configuration and nothing else.
///
/// The section headers in `.vale.ini` are globs over paths relative to the
/// config, so a fixture has to sit at a path relative to a config to be read
/// the way the gate would read it. Copying the config to a scratch root is
/// what lets `CHANGELOG.md` be probed without writing to the repo's own.
fn vale_probe_root(label: &str) -> PathBuf {
    fn copy_tree(from: &Path, to: &Path) {
        fs::create_dir_all(to).unwrap_or_else(|e| panic!("creating {}: {e}", to.display()));
        let entries =
            fs::read_dir(from).unwrap_or_else(|e| panic!("reading {}: {e}", from.display()));
        for entry in entries.flatten() {
            let source = entry.path();
            let target = to.join(entry.file_name());
            if source.is_dir() {
                copy_tree(&source, &target);
            } else {
                fs::copy(&source, &target)
                    .unwrap_or_else(|e| panic!("copying {}: {e}", source.display()));
            }
        }
    }

    let root = scratch(label);
    let config = root.join(".vale.ini");
    fs::write(&config, read(".vale.ini"))
        .unwrap_or_else(|e| panic!("writing {}: {e}", config.display()));
    copy_tree(&repo_root().join("styles"), &root.join("styles"));
    root
}

/// Run the repo's Vale configuration over one fixture written at `relative`,
/// and report whether the gate would fail on it.
fn vale_rejects(vale: &Path, root: &Path, relative: &str, content: &str) -> bool {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).unwrap_or_else(|e| panic!("creating {}: {e}", parent.display()));
    }
    fs::write(&path, content).unwrap_or_else(|e| panic!("writing {}: {e}", path.display()));
    let out = Command::new(vale)
        .arg("--output=line")
        .arg(relative)
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| panic!("running vale on {relative}: {e}"));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.trim().is_empty(),
        "vale could not read {relative}, so its verdict means nothing:\n{stderr}"
    );
    !out.status.success()
}

#[test]
fn the_prose_gate_fails_on_a_retired_name_in_a_document() {
    // The hole the change was made for, measured rather than spelled: a
    // configuration file saying `.md` is read is not the same claim as Vale
    // failing on one, and every static assertion in this section would pass on
    // a rule that never fires.
    let Some(vale) = vale_or_the_bridge_that_installs_it() else {
        return;
    };
    let (hyphen, underscore) = retired_name_in_both_spellings();
    let root = vale_probe_root("vale-documents");
    let rejected = |relative: &str, content: String| vale_rejects(&vale, &root, relative, &content);

    assert!(
        rejected("README.md", format!("# Title\n\nDelegates to {hyphen}.\n")),
        "a retired name in a Markdown paragraph is not a finding — the gate has the hole it \
         replaced"
    );
    assert!(
        rejected(
            "docs/note.md",
            format!("# Title\n\nLinks [`{underscore}`].\n")
        ),
        "a crate name in backticks is not a finding. That is how a crate name is written in a \
         Markdown document essentially always, and the underscored spelling is the one an \
         intra-doc link that outlived its crate uses — so the gate would be blind to its own \
         subject in the file kind it was added for"
    );
    assert!(
        rejected(
            "fenced.md",
            format!("# Title\n\n```rust\nuse {underscore}::thing;\n```\n"),
        ),
        "a fenced example is not a finding; an example that cannot compile is exactly the \
         rotted teaching material this rule is about, and no compiler will ever read it"
    );
    assert!(
        rejected(
            "Justfile",
            format!("# Mirrors {hyphen}'s tripwire.\nr:\n    :\n")
        ),
        "a comment in a file with no extension is not a finding; it is what the first run over \
         every tracked file turned up in the real Justfile"
    );
    assert!(
        rejected(
            ".github/workflows/ci.yml",
            format!("# was {hyphen}\non: push\n")
        ),
        "a workflow comment is not a finding"
    );

    // The control. Every row above passes on a rule that matches everything,
    // and a rule that matches everything is what a hurried fix for a false
    // positive turns this into.
    assert!(
        !rejected(
            "clean.md",
            "# Title\n\nDelegates to the sibling parser's public API.\n".to_owned(),
        ),
        "prose naming nothing retired is a finding, so the rule is not reading its token list"
    );
}

#[test]
fn the_prose_gate_still_fails_on_a_retired_name_in_a_comment() {
    // The coverage the deleted `scan_comments` tests held, asked of the tool
    // that replaced them. A move is only lossless if someone checks, and the
    // 13 unit tests that checked went with the code.
    let Some(vale) = vale_or_the_bridge_that_installs_it() else {
        return;
    };
    let (hyphen, underscore) = retired_name_in_both_spellings();
    let root = vale_probe_root("vale-comments");
    let rejected = |relative: &str, content: String| vale_rejects(&vale, &root, relative, &content);

    assert!(
        rejected("inner.rs", format!("//! Layers {hyphen} onto comrak.\n")),
        "an inner doc comment is not a finding"
    );
    assert!(
        rejected(
            "outer.rs",
            format!("    /// Delegates to [`{underscore}`].\n")
        ),
        "an outer doc comment is not a finding"
    );
    assert!(
        rejected("trailing.rs", format!("fn f() {{}} // was {hyphen}\n")),
        "a comment trailing code is not a finding, and that is where a rule records the reason \
         it is set the way it is"
    );
    assert!(
        rejected("probe.toml", format!("# a note about {hyphen}\n")),
        "a manifest comment is not a finding"
    );

    // `scope: raw` is what buys the Markdown rows in the probe above, and this
    // is the rest of what it buys. The deleted scanner left all of it alone on
    // the grounds that code which stops compiling is the compiler's business —
    // true of an `use` line, and of neither of these.
    assert!(
        rejected(
            "code.rs",
            format!("fn f() {{\n    let s = \"{hyphen}\";\n    // was {underscore}\n}}\n"),
        ),
        "a retired name in a string literal is not a finding, and no compiler is going to \
         object to it either"
    );
}

/// The globs `.vale.ini` switches the retired-path rule off for.
fn prose_gate_exemptions(config: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut section: Option<&str> = None;
    for line in config.lines() {
        let body = line.split('#').next().unwrap_or(line).trim();
        if let Some(header) = body.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            section = Some(header);
            continue;
        }
        let Some((key, value)) = body.split_once('=') else {
            continue;
        };
        if key.trim() != "Aozora.RetiredPaths" || !value.trim().eq_ignore_ascii_case("NO") {
            continue;
        }
        out.push(
            section
                .unwrap_or_else(|| panic!("`{body}` sits under no section"))
                .to_owned(),
        );
    }
    out
}

/// The paths `crates/xtask/src/main.rs` calls records rather than drift.
fn history_paths() -> Vec<String> {
    let source = read("crates/xtask/src/main.rs");
    let line = source
        .lines()
        .find(|line| line.starts_with("const HISTORY_PATHS"))
        .unwrap_or_else(|| panic!("`HISTORY_PATHS` is gone; the path gate excuses nothing now"));
    // The last `[` on the line, not the first: `&[&str]` is the type and
    // `&[...]` is the value, and a reader that stopped at the type would read
    // the exemption list as empty and pass on anything.
    let Some((items, _)) = line
        .rsplit_once('[')
        .and_then(|(_, rest)| rest.split_once(']'))
    else {
        panic!("`HISTORY_PATHS` is not a list any more: {line}")
    };
    quoted_items(items)
}

#[test]
fn the_documents_the_prose_gate_excuses_are_the_ones_the_path_gate_excuses() {
    // Two exemption lists, in two languages, written for one reason: a dated
    // document names what was removed, and rewriting it to keep a lint quiet
    // would delete the account of the decision. Nothing compares them, and an
    // exemption is the cheapest possible way to make a gate green — a
    // `docs/**` where `docs/adr/*` was meant takes the index and every future
    // design note out of the gate, and every other assertion here still
    // passes.
    let config = read(".vale.ini");
    let exemptions = prose_gate_exemptions(&config);
    assert!(
        !exemptions.is_empty(),
        "`.vale.ini` excuses nothing, so `CHANGELOG.md` cannot say what was removed"
    );

    // The rule file spells every token it bans, so it cannot be held to its
    // own rule; `RETIRED_PATH_LIST_FILE` is the same exclusion on the same
    // grounds. It is accounted for here and compared as a record nowhere.
    let styles: BTreeSet<String> = git_tracked(&["styles".to_owned()]);
    let excused: BTreeSet<String> = git_tracked(&[])
        .into_iter()
        .filter(|path| {
            exemptions
                .iter()
                .any(|glob| glob_to_regex(glob).is_match(path))
        })
        .filter(|path| !styles.contains(path))
        .collect();

    let records: BTreeSet<String> = git_tracked(&[])
        .into_iter()
        .filter(|path| {
            history_paths()
                .iter()
                .any(|history| path == history || path.starts_with(&format!("{history}/")))
        })
        .collect();

    assert!(
        records.len() > 5,
        "`HISTORY_PATHS` came out covering {records:?}; the reader is not finding the records"
    );
    assert_eq!(
        excused, records,
        "`.vale.ini` and `HISTORY_PATHS` in `crates/xtask/src/main.rs` excuse different \
         documents. The two gates split one question — a retired name in a record is not drift \
         — so a document either is a record for both of them or is drift for both of them."
    );
}

/// Vale's alert levels, weakest first. A rule reported below `MinAlertLevel`
/// is a rule the run does not fail on.
const ALERT_LEVELS: &[&str] = &["suggestion", "warning", "error"];

fn alert_rank(level: &str) -> usize {
    ALERT_LEVELS
        .iter()
        .position(|known| *known == level)
        .unwrap_or_else(|| panic!("`{level}` is not a Vale alert level"))
}

/// The `key = value` on a line of an INI file outside any section.
fn ini_preamble_value(config: &str, key: &str) -> Option<String> {
    for line in config.lines() {
        let body = line.split('#').next().unwrap_or(line).trim();
        if body.starts_with('[') {
            return None;
        }
        if let Some((name, value)) = body.split_once('=')
            && name.trim() == key
        {
            return Some(value.trim().to_owned());
        }
    }
    None
}

#[test]
fn the_prose_rule_is_written_at_a_level_that_makes_the_gate_exit_non_zero() {
    // The `doc` gate's defect, in the file where it is one edit away: Vale
    // exits 0 when every alert it raised is below `MinAlertLevel`. A rule
    // demoted to `warning` still prints, still looks like a gate in the log,
    // and fails nothing — and the demotion is exactly what a hurried fix for a
    // false positive looks like. `just vale` would go on passing, and so would
    // every assertion above, which all ask what Vale reads and none of which
    // asks what Vale does about it.
    let config = read(".vale.ini");
    let floor = ini_preamble_value(&config, "MinAlertLevel")
        .unwrap_or_else(|| panic!("`.vale.ini` sets no `MinAlertLevel`"));

    let dir = repo_root().join("styles").join("Aozora");
    let entries = fs::read_dir(&dir).unwrap_or_else(|e| panic!("reading {}: {e}", dir.display()));
    let mut rules = 0usize;
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("yml") {
            continue;
        }
        let rule =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        let Some(declared) = rule.lines().find_map(|line| line.strip_prefix("level:")) else {
            panic!("{} declares no `level:`", path.display())
        };
        let level = declared.trim();
        rules += 1;
        assert!(
            alert_rank(level) >= alert_rank(&floor),
            "{} reports at `{level}`, which is under this configuration's `MinAlertLevel = \
             {floor}`. Vale prints the alert and exits 0, so `just vale` passes on the finding \
             it was written to make.",
            path.display()
        );
    }
    assert!(rules > 0, "there is no rule in `styles/Aozora` to run");
}

/// The `if:` conditions that decide whether a JOB runs, as opposed to a step.
/// The key is the same at both depths and only the outer one can take a whole
/// job out of the run, so the depth is what separates them here. The inner one
/// is not thereby harmless — it is read by [`step_condition`] and held to
/// [`CONDITIONAL_GATE_STEPS_IN_CI`], because a skipped step leaves the job
/// reporting success.
fn job_level_conditions(workflow: &str, job: &str) -> Vec<String> {
    let lines =
        job_lines(workflow, job).unwrap_or_else(|| panic!("ci.yml has no `{job}:` job any more"));
    lines
        .iter()
        .filter(|line| line.starts_with("    ") && !line.starts_with("     "))
        .filter_map(|line| strip_comment(line).trim().strip_prefix("if:"))
        .map(|condition| condition.trim().to_owned())
        .collect()
}

/// The steps of one job, each as its own lines.
///
/// Kept apart rather than read out of the job whole: an `if:` belongs to the
/// step it is written in, and the question below is which step it takes out of
/// the run. A reader handed the job would answer for a neighbour's condition —
/// `commitlint` has one on its tool install and one on its gate, and they are
/// not the same fact.
fn job_steps<'a>(workflow: &'a str, job: &str) -> Vec<Vec<&'a str>> {
    let lines =
        job_lines(workflow, job).unwrap_or_else(|| panic!("ci.yml has no `{job}:` job any more"));
    let mut out: Vec<Vec<&str>> = Vec::new();
    let mut at: Option<usize> = None;
    for line in nested_block(&lines, "steps") {
        let body = strip_comment(line);
        let text = body.trim_start();
        if text.is_empty() {
            continue;
        }
        let indent = body.len() - text.len();
        // A `- ` deeper in is a list item inside a step (`with:` inputs, a
        // shell heredoc); only one at the column the first step opened starts
        // another step.
        if text.starts_with("- ") && at.is_none_or(|first| indent == first) {
            at = Some(indent);
            out.push(Vec::new());
        }
        if let Some(step) = out.last_mut() {
            step.push(line);
        }
    }
    out
}

/// The `if:` that decides whether one step runs.
///
/// Read at the step's own key column, which is the `- ` column plus two. A
/// `run: |` script says `if [[ ... ]]` in the middle of a step and means
/// nothing about whether the step runs, so a substring match would report a
/// condition where there is none — and a rule that fires on shell would be
/// carved out until it fired on nothing.
fn step_condition(step: &[&str]) -> Option<String> {
    let head = strip_comment(step.first()?);
    let column = head.len() - head.trim_start().len() + 2;
    for line in step {
        let body = strip_comment(line);
        let text = body.trim_start();
        let indent = body.len() - text.len();
        let (indent, text) = text
            .strip_prefix("- ")
            .map_or((indent, text), |rest| (indent + 2, rest));
        if indent == column
            && let Some(condition) = text.strip_prefix("if:")
        {
            return Some(condition.trim().to_owned());
        }
    }
    None
}

/// The conditions written on steps that run a recipe, in one job.
///
/// A `run:` naming `just`, whatever it hands it: the matrix leg runs
/// `just "$GATE"`, so a reader that wanted a recipe name would have skipped
/// the one step that is every gate at once. `uses:` steps are out — the
/// `tool: committed,just` install puts the runner on the box and runs nothing.
fn conditional_recipe_steps(workflow: &str, job: &str) -> Vec<String> {
    job_steps(workflow, job)
        .into_iter()
        .filter(|step| {
            step.iter().any(|line| strip_comment(line).contains("run:"))
                && step
                    .iter()
                    .any(|line| words(strip_comment(line)).any(|word| word == "just"))
        })
        .filter_map(|step| step_condition(&step))
        .collect()
}

/// Where a condition is written, which is what decides what a skip looks
/// like from outside: a skipped job reports "skipped", a skipped step leaves
/// its job reporting success.
#[derive(Clone, Copy)]
enum Depth {
    Job,
    Step,
}

impl fmt::Display for Depth {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Job => "job",
            Self::Step => "gate step",
        })
    }
}

/// Every `if:` in ci.yml that can take a gate out of the run, at either depth.
fn conditions_that_can_skip_a_gate(workflow: &str) -> Vec<(String, String, Depth)> {
    let mut out = Vec::new();
    for job in job_keys(workflow) {
        for condition in job_level_conditions(workflow, &job) {
            out.push((job.clone(), condition, Depth::Job));
        }
        for condition in conditional_recipe_steps(workflow, &job) {
            out.push((job.clone(), condition, Depth::Step));
        }
    }
    out
}

/// Lines that use an action whose job is to sort a diff.
///
/// A list of names, and the weaker of the two readings for exactly that
/// reason: an action nobody has heard of leaves nothing to grep for. It earns
/// its place on the case the glob reader cannot see — an action pointed at a
/// filter file, whose globs are not in this workflow at all.
fn path_classifier_actions(workflow: &str) -> Vec<&str> {
    const SORTERS: &[&str] = &["paths-filter", "changed-files"];
    jobs_block(workflow)
        .into_iter()
        .filter(|line| {
            let body = strip_comment(line);
            SORTERS.iter().any(|action| {
                body.split_whitespace()
                    .any(|word| word.contains(action) && word.contains('/'))
            })
        })
        .collect()
}

/// `paths:` / `paths-ignore:` written on a trigger rather than inside a job.
fn trigger_path_filters(workflow: &str) -> Vec<&str> {
    top_level_block(workflow, "on")
        .into_iter()
        .filter(|line| {
            let key = strip_comment(line).trim().trim_end_matches(':');
            key == "paths" || key == "paths-ignore"
        })
        .collect()
}

/// Every job in ci.yml whose execution is conditional, with the condition and
/// the reason it is not a diff being sorted out of the run.
///
/// The list is the rule: a job-level `if:` that is not written here fails,
/// whatever it says. The `changes` filter was three of these at once, and each
/// read plausibly on its own line.
const CONDITIONAL_JOBS_IN_CI: &[(&str, &str, &str)] = &[
    (
        "commitlint",
        "github.event_name == 'pull_request'",
        "the event kind, not the diff. The recipe lints a base..head range, and a push to main \
         has no such range — the job is skipped where its question does not exist, not where \
         somebody decided the answer was cheap.",
    ),
    (
        "ci-success",
        "always()",
        "the aggregator, and the only condition here that widens rather than narrows: without it \
         a failed upstream job would leave the one required check unreported, which reads as \
         pending forever instead of red.",
    ),
];

/// Every step in ci.yml that runs a recipe and does not always run.
///
/// One row, and it is a hole rather than an exemption. A conditional step is
/// the `changes` filter one level down and strictly worse: the job it sits in
/// still reports SUCCESS, so `ci-success` goes green over a gate that never
/// executed, where a skipped job at least shows as skipped.
const CONDITIONAL_GATE_STEPS_IN_CI: &[(&str, &str, &str)] = &[(
    "commitlint",
    "github.event.pull_request.user.login != 'dependabot[bot]'",
    "the one gate this repo knowingly does not run, and it is written down because it is not \
     free: a bot PR merges with its commit subjects unlinted, on the grounds that the squash \
     subject a maintainer writes is what actually lands on main.",
)];

#[test]
fn nothing_classifies_a_diff_into_a_run_with_no_gates_in_it() {
    // A gate that cannot be reached is a gate that does not exist, and for a
    // year the only thing in this repo that could decide a gate was not
    // reached was a `changes` job: dorny/paths-filter, one `rust` boolean, and
    // the whole compile/test/lint matrix hanging off it. It classified a
    // Markdown-only diff as "not rust" and skipped everything — so the prose
    // gate, added because no gate could open a `.md` file, would not have run
    // on a change to one. Widening the filter to `**/*.md` (#208) fixed that
    // file kind and left the shape: three gates read every tracked file with
    // no pathspec at all, `spec` and `dist-assets-check` read whole trees of
    // their own, and the filter still watched a hand-written subset. Twenty-
    // seven tracked files were left that a diff could hide behind, the
    // sharpest of them `.gitattributes` — the repo-path scan exists BECAUSE a
    // retired path survived in that file, and a change to it skipped the
    // matrix that runs the scan.
    //
    // The filter is gone (#210), and this is the direction that keeps it gone.
    // Four ways back in: the classifier itself, a `paths:` on a trigger, a job
    // that runs only under some condition, and a STEP that does. The last one
    // is the sharpest and was the one this rule first left open — a skipped
    // job reports "skipped", a skipped step leaves its job reporting success.
    //
    // The two condition rules are lists rather than shapes on purpose. Reading
    // the condition and judging it — banning `needs.`, say — passes anything
    // spelled a different way, and the ways are not enumerable: a label, a
    // commit subject, an output of a step in the same job. What is enumerable
    // is the conditions this file is meant to have.
    let workflow = read(".github/workflows/ci.yml");
    let jobs = job_keys(&workflow);
    assert!(
        jobs.len() >= 5,
        "ci.yml came out with jobs {jobs:?}; the reader is not finding the `jobs:` mapping"
    );

    // Asked twice, because either half alone names a vendor or a spelling.
    let classifiers = path_classifier_actions(&workflow);
    assert!(
        classifiers.is_empty(),
        "ci.yml classifies the diff again: {classifiers:?}\n\
         `just vale`, `just typos` and `just comment-discipline` read every tracked file, so the \
         only filter that is right for them is no filter — and the one this replaced skipped \
         exactly the files it should not have."
    );
    assert!(
        diff_classifier(&workflow).is_empty(),
        "ci.yml carries globs that sort a diff. Which files they name is the next rule's \
         question; this one is that sorting a diff at all is what was decided against — a filter \
         wide enough to be harmless is a filter somebody will narrow."
    );

    // The same skip spelled at the trigger instead of in a job, where nothing
    // downstream would show it: a workflow that never starts reports nothing
    // at all, and branch protection is satisfied by a check that never ran.
    let trigger_paths = trigger_path_filters(&workflow);
    assert!(
        trigger_paths.is_empty(),
        "ci.yml's `on:` block filters by path: {trigger_paths:?}\n\
         A diff outside the list does not skip the matrix, it skips the workflow — so there is \
         no `ci-success` to be required at all."
    );

    let mut conditional: BTreeSet<(String, String)> = BTreeSet::new();
    for (job, condition, depth) in conditions_that_can_skip_a_gate(&workflow) {
        let (accounted, list, cost) = match depth {
            Depth::Job => (
                CONDITIONAL_JOBS_IN_CI,
                "CONDITIONAL_JOBS_IN_CI",
                "A gate something can answer away is a gate: the `changes` filter was three of \
                 these, and `if: always()` on the aggregator is why every one of them stayed \
                 green.",
            ),
            Depth::Step => (
                CONDITIONAL_GATE_STEPS_IN_CI,
                "CONDITIONAL_GATE_STEPS_IN_CI",
                "The step is skipped and the job it is in succeeds, so the check reports green \
                 having executed nothing — the #210 defect with the evidence removed.",
            ),
        };
        assert!(
            accounted
                .iter()
                .any(|(named, allowed, _)| *named == job && *allowed == condition),
            "ci.yml's `{job}` {depth} runs only when `{condition}`, which is not a condition this \
             workflow is meant to carry. {cost} Add it to {list} with what it costs, or take the \
             condition off."
        );
        conditional.insert((job, condition));
    }

    // Both lists the other way round. A row for a condition that is no longer
    // written is a rule that has stopped being a measurement — and it is the
    // shape that lets the next one in, since a list nobody has to keep true is
    // a list nobody reads before adding to.
    let dead: Vec<String> = CONDITIONAL_JOBS_IN_CI
        .iter()
        .chain(CONDITIONAL_GATE_STEPS_IN_CI)
        .filter(|(job, condition, _)| {
            !conditional.contains(&((*job).to_owned(), (*condition).to_owned()))
        })
        .map(|(job, condition, _)| format!("{job}: `{condition}`"))
        .collect();
    assert!(
        dead.is_empty(),
        "these jobs are accounted for as conditional and are not: {dead:?}\n\
         Drop the row, so the list stays the account of this workflow it claims to be."
    );
}

/// Three steps, and every way the step reader can be wrong in one job: a `- `
/// nested inside a step, a shell `if` inside a `run:` script, and a condition
/// on the step after the one it would be blamed on.
const STEPS_THE_READER_HAS_TO_TELL_APART: &str = concat!(
    "jobs:\n",
    "  gate:\n",
    "    steps:\n",
    "      - uses: actions/checkout@0000000\n",
    "        with:\n",
    "          persist-credentials: false\n",
    "      - name: Read the gate manifest\n",
    "        run: |\n",
    "          if [[ \"$native\" != \"commitlint\" ]]; then\n",
    "            exit 1\n",
    "          fi\n",
    "          just gates\n",
    "      - name: Run the gate\n",
    "        if: steps.filter.outputs.rust == 'true'\n",
    "        run: just \"$GATE\"\n",
);

#[test]
fn a_step_condition_is_read_off_its_own_step_and_not_a_neighbours() {
    // The rule above is only as good as where it thinks a step starts and
    // ends. Two ways it could report nothing on the workflow that has the
    // defect: reading `if [[ ... ]]` in a script as a condition makes the rule
    // fire on an honest file until somebody carves it out, and blaming the
    // wrong step makes it name a step that would pass review.
    let steps = job_steps(STEPS_THE_READER_HAS_TO_TELL_APART, "gate");
    assert_eq!(
        steps.len(),
        3,
        "the reader split the job into {} steps; `with:` inputs and a shell script are inside a \
         step, not steps of their own",
        steps.len()
    );
    assert_eq!(step_condition(&steps[0]), None);
    assert_eq!(
        step_condition(&steps[1]),
        None,
        "a shell `if` in a `run:` script was read as the step's own condition"
    );
    assert_eq!(
        step_condition(&steps[2]).as_deref(),
        Some("steps.filter.outputs.rust == 'true'"),
        "the condition that takes the whole gate matrix out of the run was not read"
    );
}

/// What ci.yml sorts a diff with: the globs that make a diff worth running
/// for, and the globs that make one not.
struct DiffClassifier {
    watched: Vec<Regex>,
    ignored: Vec<Regex>,
}

impl DiffClassifier {
    /// Does this workflow sort a diff at all?
    ///
    /// Not the same question as whether anything is hidden today. A filter
    /// widened until it watches everything hides nothing and is still a filter
    /// — the state this one was in when it was deleted, with the wrong 27
    /// files left in it.
    fn is_empty(&self) -> bool {
        self.watched.is_empty() && self.ignored.is_empty()
    }

    /// Would a diff touching only `path` be sorted out of the run?
    ///
    /// Both halves, because the two spellings answer opposite ways: an
    /// unmatched glob hides a file only when there is an inclusive list to be
    /// outside of, and a matched one hides it only when the list is the
    /// `-ignore` kind. A reader that folded them together would call a
    /// `paths-ignore` of the whole tree "everything is watched".
    fn hides(&self, path: &str) -> bool {
        if self.ignored.iter().any(|glob| glob.is_match(path)) {
            return true;
        }
        !self.watched.is_empty() && !self.watched.iter().any(|glob| glob.is_match(path))
    }
}

/// Every glob ci.yml classifies a diff with, in every spelling one can be
/// written in: `paths:` / `paths-ignore:` on a trigger, and the `filters:` or
/// `files:` a path-classifier action takes as its input.
///
/// One reader for all of them because it is one question — which diffs this
/// workflow decides are worth running for — and because naming the action
/// that asks it is how a rule ends up meaning "not that vendor" instead of
/// "not this".
fn diff_classifier(workflow: &str) -> DiffClassifier {
    const WATCHES: &[&str] = &["paths", "filters", "files"];
    const IGNORES: &[&str] = &["paths-ignore", "files-ignore"];
    let mut watched = Vec::new();
    let mut ignored = Vec::new();
    let mut opened: Option<(usize, bool)> = None;
    for raw in workflow.lines() {
        let body = strip_comment(raw);
        let text = body.trim();
        if text.is_empty() {
            continue;
        }
        let indent = body.len() - body.trim_start().len();
        if let Some((column, subtracts)) = opened {
            if indent > column {
                // A list item, or — where the action takes its globs as one
                // newline-separated scalar rather than a list — a line that is
                // not a mapping key. `filters:` nests its globs under a name
                // and `files:` does not, so a reader that knew only the list
                // form would read one vendor's classifier and be blind to the
                // next one's.
                let item = match text.strip_prefix("- ") {
                    Some(rest) => Some(rest.trim()),
                    None if text.ends_with(':') || text.contains(": ") => None,
                    None => Some(text),
                };
                if let Some(item) = item {
                    // A `!glob` subtracts from what a filter watches, so it
                    // belongs with the other spelling of "not this one".
                    let item = unquoted(item);
                    match item.strip_prefix('!') {
                        Some(negated) => ignored.push(negated.to_owned()),
                        None if subtracts => ignored.push(item),
                        None => watched.push(item),
                    }
                }
                continue;
            }
            opened = None;
        }
        let Some((key, rest)) = text.split_once(':') else {
            continue;
        };
        let subtracts = IGNORES.contains(&key);
        if !subtracts && !WATCHES.contains(&key) {
            continue;
        }
        // `filters: |` opens a block scalar and `paths:` a plain list; either
        // way the globs are the items indented under it.
        let rest = rest.trim().trim_start_matches(['|', '>', '-', '+']).trim();
        if rest.is_empty() {
            opened = Some((indent, subtracts));
            continue;
        }
        let Some(items) = rest
            .strip_prefix('[')
            .and_then(|list| list.strip_suffix(']'))
        else {
            continue;
        };
        for item in items
            .split(',')
            .map(|item| unquoted(item.trim()))
            .filter(|item| !item.is_empty())
        {
            if subtracts || item.starts_with('!') {
                ignored.push(item.trim_start_matches('!').to_owned());
            } else {
                watched.push(item);
            }
        }
    }
    let compile = |globs: Vec<String>| globs.iter().map(|glob| glob_to_regex(glob)).collect();
    DiffClassifier {
        watched: compile(watched),
        ignored: compile(ignored),
    }
}

/// A YAML scalar with its quotes off, if it had any.
fn unquoted(value: &str) -> String {
    quoted_literal(value, '\'')
        .or_else(|| quoted_literal(value, '"'))
        .unwrap_or(value)
        .to_owned()
}

/// The classifier this repository actually shipped, kept as the reader's
/// control. Both spellings are in it: the trigger form, which skips the
/// workflow outright, and the action form, which skipped every job that hung
/// off its one boolean.
const CLASSIFIER_THAT_WAS_DELETED: &str = concat!(
    "on:\n",
    "  pull_request:\n",
    "    branches: [main]\n",
    "    paths: ['spec/**']\n",
    "\n",
    "jobs:\n",
    "  changes:\n",
    "    steps:\n",
    "      - uses: dorny/paths-filter@7b450fff21473bca461d4b92ce414b9d0420d706  # v4.0.2\n",
    "        id: filter\n",
    "        with:\n",
    "          filters: |\n",
    "            rust:\n",
    "              - 'crates/**'\n",
    "              - 'Cargo.toml'\n",
    "              - '.github/**'\n",
    "              - '**/*.md'\n",
);

/// The same classification by another vendor, whose globs are a newline-
/// separated scalar and not a list. The rule is "nothing sorts a diff", not
/// "not that action" — and this is the shape the first reader was blind to.
const THE_SAME_SORTING_BY_ANOTHER_HAND: &str = concat!(
    "jobs:\n",
    "  changes:\n",
    "    steps:\n",
    "      - uses: tj-actions/changed-files@0000000\n",
    "        id: sorted\n",
    "        with:\n",
    "          files: |\n",
    "            crates/**\n",
    "            **/*.md\n",
);

#[test]
fn a_diff_sorted_out_of_the_run_is_one_the_reader_reports() {
    // The rule below answers "nothing is hidden", and the cheapest way for it
    // to answer that is to have found nothing to read. This is what says it
    // did not: the same reader, over the classifier that was actually here,
    // reporting the same files the issue counted.
    let classifier = diff_classifier(CLASSIFIER_THAT_WAS_DELETED);
    for hidden in [
        ".gitattributes",
        "NOTICE",
        "lefthook.yml",
        "bin/aozora-flavored-markdown",
        ".config/mise/config.toml",
    ] {
        assert!(
            classifier.hides(hidden),
            "the reader says a diff touching only `{hidden}` reached the gate matrix under the \
             filter that skipped it. Reading nothing looks exactly like this."
        );
    }
    // `spec/**` is watched by the trigger and by nothing else, so it is the
    // one that says both spellings were read and not just the first.
    for watched in [
        "spec/commonmark-0.31.2.json",
        "Cargo.toml",
        "README.md",
        "crates/xtask/src/main.rs",
        ".github/workflows/ci.yml",
    ] {
        assert!(
            !classifier.hides(watched),
            "the reader says `{watched}` was hidden by a filter that watched it — it is reporting \
             more than it read, which would make the rule below pass on a filter that is really \
             there."
        );
    }

    let elsewhere = diff_classifier(THE_SAME_SORTING_BY_ANOTHER_HAND);
    assert!(
        !elsewhere.is_empty() && elsewhere.hides(".gitattributes"),
        "a classifier written as a newline-separated scalar read as no classifier at all. Every \
         rule here would pass on the workflow that has the defect, which is how a net comes to \
         mean `not that vendor` instead of `not this`."
    );
}

#[test]
fn no_tracked_file_can_hide_a_diff_from_the_gate_matrix() {
    // Acceptance, stated over files rather than over mechanisms: a diff
    // touching only `.gitattributes` runs the gate matrix. The mechanisms are
    // held above; this is the thing they were held for, and it is the half a
    // reader can check against the repository rather than against the
    // workflow's own vocabulary.
    //
    // Twenty-seven tracked files failed this before #210 — `spec/`, `dist/`,
    // every per-gate configuration at the root, `lefthook.yml`, `.config/`,
    // `bin/`, `.devcontainer/`, `.editorconfig`, `.gitattributes`,
    // `.gitignore`, the licences and `NOTICE`. Each one is read by a gate that
    // would have failed on it locally. The issue counted 26 by hand, which is
    // the other half of what this rule replaces: the set was written down once
    // and is measured now.
    let tracked = git_tracked(&[]);
    assert!(
        tracked.contains(".gitattributes"),
        "the repository no longer tracks `.gitattributes`, so this rule is measuring a tree that \
         does not contain the file it was written for"
    );

    let classifier = diff_classifier(&read(".github/workflows/ci.yml"));
    let hidden: Vec<&String> = tracked
        .iter()
        .filter(|path| classifier.hides(path))
        .collect();
    let shown: Vec<&&String> = hidden.iter().take(20).collect();
    assert!(
        hidden.is_empty(),
        "a diff touching only one of these {} tracked files skips the gate matrix: {shown:?}\n\
         `just vale`, `just typos` and `just comment-discipline` read every tracked file with no \
         pathspec at all, and `just spec` and `just dist-assets-check` own whole trees, so a gate \
         can fail locally on a file CI never ran it over.",
        hidden.len()
    );
}

// ---------------------------------------------------------------------------
// a check nothing ran, and the sentence that kept it that way
// ---------------------------------------------------------------------------
//
// The sixth way, and the first way over again with the one list nobody was
// reading. `just semver` was written, tagged `[group('lint')]`, and invoked by
// nothing: not by `just lint`, which bundles its dependencies by hand, and not
// by CI, which expands `[group('gate')]`. Every rule at the top of this file
// passed on it. `every_tool_this_repo_declares_is_a_tool_this_repo_runs` asks
// the question in the right words and asks it of `mise.toml`, so a tool the
// image installs and a recipe drives is out of its reach. And
// `every_lint_the_bundle_runs_is_a_gate_the_manifest_declares` compares the
// bundle against the manifest, so a recipe in neither is in neither
// difference. The group attribute — the one place that said "this is a lint"
// — was read by nothing at all.
//
// What it cost is the whole public-surface rebuild: every entry point, every
// IR name and every error type moved with the one tool that measures such a
// move sitting idle. And the reason it sat idle was written down as policy,
// in an accepted ADR, in a document class this file had excused as history:
// "`cargo semver-checks` cannot run until a baseline exists on crates.io". It
// was never true — `--baseline-rev` resolves out of git — but it read like a
// decision, so it was treated as one. That is SECURITY.md's "there is no cron
// workflow" again, one document class further out.
//
// So: the manifest has to reach every recipe that calls itself a check, no
// current document may deny that a check this repo owns can run, and the gate
// itself has to be held to what it now asserts — a baseline it can resolve, an
// exclusion list no larger than the crates that baseline predates, and
// arguments measured to fail on a break rather than trusted to.

/// The `[group('lint')]` recipes no `[group('gate')]` declares, minus the two
/// shapes in that group that are not checks: an aggregate (dependencies and no
/// command of its own) and the writing half of a pair whose `-check` half is
/// gated.
fn checks_no_gate_runs(justfile: &str) -> Vec<String> {
    let gates = recipes_in_group(justfile, "gate");
    recipes_in_group(justfile, "lint")
        .into_iter()
        .filter(|recipe| !gates.contains(recipe))
        .filter(|recipe| !gates.contains(&format!("{recipe}-check")))
        .filter(|recipe| {
            joined_body(justfile, recipe)
                .iter()
                .any(|line| !line.trim().is_empty())
        })
        .collect()
}

#[test]
fn every_check_this_repo_declares_is_one_a_gate_runs() {
    let justfile = read("Justfile");
    let declared = recipes_in_group(&justfile, "lint");
    assert!(
        declared.len() >= 10,
        "`[group('lint')]` came out as {declared:?}; the reader is not finding the attribute"
    );

    let idle = checks_no_gate_runs(&justfile);
    assert!(
        idle.is_empty(),
        "declared a lint and gated by nothing: {idle:?}\n\
         `[group('lint')]` is this repo saying a recipe is a check. `[group('gate')]` is the \
         only thing that makes one run — `just ci` and the CI matrix both read it and nothing \
         else. A recipe in the first group and not the second runs when somebody remembers, \
         which for `just semver` was never. Tag it, or take the lint group off it."
    );
}

#[test]
fn the_lint_group_as_it_stood_held_a_check_nothing_ran() {
    // The `Justfile` as it was. Every other rule in this file passed on it:
    // the recipe is not in `just lint`'s dependency list, so the bundle
    // comparison never saw it, and it is not in the manifest, so neither did
    // anything reading that.
    let before = concat!(
        "[group('lint')]\n",
        "lint: fmt-check clippy\n",
        "\n",
        "[group('gate')]\n",
        "[group('lint')]\n",
        "fmt-check:\n",
        "    cargo fmt --all -- --check\n",
        "\n",
        "[group('lint')]\n",
        "fmt:\n",
        "    cargo fmt --all\n",
        "\n",
        "[group('lint')]\n",
        "semver:\n",
        "    cargo semver-checks check-release --workspace\n",
    );
    assert_eq!(
        checks_no_gate_runs(before),
        vec!["semver".to_owned()],
        "the reader no longer sees the defect it was written for. `lint` is the bundle and \
         `fmt` is `fmt-check`'s writing half; `semver` is the check nothing ran."
    );

    let after = before.replace(
        "[group('lint')]\nsemver:",
        "[group('gate')]\n[group('lint')]\nsemver:",
    );
    assert!(
        checks_no_gate_runs(&after).is_empty(),
        "tagging the recipe is what settles it: {:?}",
        checks_no_gate_runs(&after)
    );
}

// ---------------------------------------------------------------------------
// prose that denies a check this repo can run
// ---------------------------------------------------------------------------

/// Denials of capability, as opposed to [`NEGATIONS`]' denials of fact. The
/// two are answered by different evidence: "this does not run" is a claim
/// about the manifest, and "this cannot run" is a claim about a tool, refuted
/// by the repo owning a recipe that runs it.
const CAPABILITY_DENIALS: &[&str] = &["cannot", "unable", "impossible"];

/// The nouns a denial of a run attaches to.
const RUN_NOUNS: &[&str] = &[
    "run",
    "runs",
    "ran",
    "running",
    "gate",
    "gates",
    "gated",
    "ci",
    "pr",
    "per-pr",
    "pull-request",
    "enforced",
    "wired",
    "invoked",
];

/// The tools this repo drives, as a document would spell them: the
/// sub-command for a `cargo` call, the command name for anything else.
///
/// Every recipe, not only the gated ones, and that is the point. The claim
/// being refuted is "this tool cannot run here", and a recipe that runs it is
/// the refutation whether or not anything runs the recipe — which for
/// `cargo semver-checks` was the entire defect.
fn tool_vocabulary(justfile: &str) -> BTreeSet<String> {
    let variables = plain_variables(justfile);
    let mut out = BTreeSet::new();
    for (_, line) in expanded_recipe_lines(justfile) {
        for (tool, sub) in tool_commands(&expand(&line, &variables)) {
            out.insert(sub);
            if tool != "cargo" {
                out.insert(tool);
            }
        }
    }
    out
}

/// The names from `vocabulary` that `clause` puts in COMMAND position: a
/// backticked span read as a command line, minus the driver that opens it.
///
/// Position rather than spelling. Reading bare words meant subtracting a
/// hand-written list of nineteen tool names that are also ordinary English —
/// `build`, `check`, `run`, `test`, `doc`, `spec` — and every one of the
/// nineteen was then invisible to the rule. Markup settles the same question
/// without a list, because it is what the list was approximating: a document
/// discussing a build writes build, one naming the recipe writes `just build`.
///
/// The drivers are [`BUILD_TOOLS`] — already this file's answer to "which
/// invocation IS the call" — plus `just`, which runs the recipes. A path is
/// read to its first flag or argument, so `cargo xtask comment-discipline`
/// names the sub-command two deep that a `<tool> <sub>` pair could not reach,
/// and `cargo test --features build` still does not name `build`.
fn names_in_command_position(clause: &str, vocabulary: &BTreeSet<String>) -> Vec<String> {
    let mut named = BTreeSet::new();
    for span in clause.split('`').skip(1).step_by(2) {
        let path: Vec<String> = span
            .split_whitespace()
            .take_while(|token| is_subcommand_word(token))
            .map(str::to_lowercase)
            .collect();
        // A lone backticked word is its own command path. Anything longer is
        // read only if it opens as a call, which is also what keeps a
        // backticked quotation of prose from naming everything inside it.
        let mut driven = path.len() == 1;
        for word in &path {
            if driven && vocabulary.contains(word) {
                named.insert(word.clone());
            }
            driven |= BUILD_TOOLS.contains(&word.as_str()) || word == "just";
        }
    }
    named.into_iter().collect()
}

#[test]
fn no_document_denies_a_check_this_repo_can_run() {
    // That the vocabulary still holds `semver-checks` — that this is reading
    // the recipes at all — is asserted by the reader's own test below, which
    // takes the sentence this rule was written for all the way to the name.
    let justfile = read("Justfile");
    let tools = tool_vocabulary(&justfile);
    let gates = recipes_in_group(&justfile, "gate");
    let documents = ci_prose_files();

    let mut claims = Vec::new();
    for (label, text) in &documents {
        for (line, denial) in denials_in(text, CAPABILITY_DENIALS, RUN_NOUNS) {
            let named = names_in_command_position(&denial, &tools);
            if !named.is_empty() {
                claims.push(format!(
                    "  {label}:{line}: {denial}\n    a recipe here runs {named:?}"
                ));
            }
        }
        for (line, denial) in denials_in(text, NEGATIONS, RUN_NOUNS) {
            let named = names_in_command_position(&denial, &gates);
            if !named.is_empty() {
                claims.push(format!(
                    "  {label}:{line}: {denial}\n    {named:?} is in the gate manifest"
                ));
            }
        }
    }
    assert!(
        claims.is_empty(),
        "prose that denies a check this repo can run:\n{}\n\
         A document in force saying a tool cannot run, or that a gate is not gated, is not a \
         description that has gone stale — it is an instruction not to look. ADR-0015's \
         `cargo semver-checks` sentence held the public-surface rebuild open for exactly as \
         long as it took somebody to disbelieve it.",
        claims.join("\n")
    );
}

#[test]
fn a_denial_is_read_where_it_is_made_and_not_where_the_line_happens_to_wrap() {
    // Run at the scope the rules run at — a whole document. The two readers
    // this replaced were asked one line at a time, a different reach and a
    // different quote scope from the call they stood in for, so they agreed
    // with a rule whose contract they did not share.
    //
    // ADR-0015's sentence, wrapped as the ADR wrapped it, and again with the
    // wrap moved one clause along. The old reader saw the first and not the
    // second: the tool and the denial had to land on the same physical line.
    let tools = tool_vocabulary(&read("Justfile"));
    for claim in [
        concat!(
            "`cargo semver-checks` cannot run until a baseline exists on crates.io, so it\n",
            "is wired into the `publish-crates.yml` preflight *after* the first publish,\n",
            "not into per-PR CI.\n",
        ),
        concat!(
            "Until a baseline exists on crates.io, `cargo semver-checks`\n",
            "cannot run at all.\n",
        ),
    ] {
        let found = denials_in(claim, CAPABILITY_DENIALS, RUN_NOUNS);
        assert_eq!(
            found.len(),
            1,
            "the reader no longer sees the sentence it was written for: {found:?}"
        );
        assert_eq!(
            names_in_command_position(&found[0].1, &tools),
            vec!["semver-checks".to_owned()],
            "the denial no longer reads as being about a tool this repo drives"
        );
    }

    // The sentence the schedule half exists for; a denial whose negation is
    // interrupted rather than ended by its commas, which a clause cut at every
    // one of them put out of reach; then two that describe what this repo
    // runs: CONTRIBUTING.md's two native gates, whose negation is handed over
    // by a conjunction and which name both gates in command position, so a
    // paragraph read whole would convict it; and a pasted transcript, which is
    // evidence rather than a claim.
    for (prose, nouns, denials) in [
        (
            "Both ride every PR; there is no cron workflow.\n",
            SCHEDULE_NOUNS,
            1,
        ),
        ("`just test` does not, on a PR, run.\n", RUN_NOUNS, 1),
        (
            concat!(
                "The two `[group('native')]` gates (`msrv`, `commitlint`) keep a dedicated CI\n",
                "job because they need a toolchain the dev image has not got, and they run\n",
                "the same recipe there.\n",
            ),
            RUN_NOUNS,
            0,
        ),
        (
            concat!(
                "It printed:\n\n",
                "```text\n",
                "there is no cron workflow\n",
                "```\n"
            ),
            SCHEDULE_NOUNS,
            0,
        ),
    ] {
        let found = denials_in(prose, NEGATIONS, nouns);
        assert_eq!(found.len(), denials, "{prose:?} was read as {found:?}");
    }
}

// ---------------------------------------------------------------------------
// the baseline the semver gate compares against
// ---------------------------------------------------------------------------
//
// Wiring the recipe up is one claim; the arguments in it are another. Three
// ways this gate can run green over nothing, none of which any rule above
// reaches. It can name a baseline the checkout cannot resolve, which fails
// loudly in the one shape that reads like the tool being broken rather than
// the API having moved. It can `--exclude` its way past a crate the baseline
// does hold, which is the coverage-denominator defect in another file. And it
// can pass because the version already declares a major bump, which is a
// correct answer to a question nobody wanted asked.

/// The recipe that compares this workspace's public API against a baseline.
const SEMVER_GATE: &str = "semver";

/// The arguments the [`SEMVER_GATE`] recipe hands `cargo semver-checks`.
/// Continuations folded, run prefix and any unresolved `{{…}}` dropped.
fn semver_arguments(justfile: &str) -> Vec<String> {
    let tokens: Vec<String> = joined_body(justfile, SEMVER_GATE)
        .iter()
        .flat_map(|line| shell_tokens(line))
        .filter(|token| !token.starts_with("{{"))
        .collect();
    let at = tokens
        .iter()
        .position(|token| token == "semver-checks")
        .unwrap_or_else(|| {
            panic!("`just {SEMVER_GATE}` no longer runs `cargo semver-checks`: {tokens:?}")
        });
    tokens[at + 1..].to_vec()
}

/// Every value `flag` is given, in order. `--exclude` is repeated once per
/// crate, so the first hit is not the answer.
fn flag_values<'a>(arguments: &'a [String], flag: &str) -> Vec<&'a str> {
    arguments
        .iter()
        .zip(arguments.iter().skip(1))
        .filter(|(name, _)| name.as_str() == flag)
        .map(|(_, value)| value.as_str())
        .collect()
}

/// Does this flag's value name a git revision? `--baseline-rev` does.
/// criterion's `--baseline`, which names a saved benchmark run and sits in the
/// `bench-compare` recipe of this same file, does not — and a reader that took
/// the word `baseline` for the question would have said it did.
fn takes_a_revision(flag: &str) -> bool {
    flag.starts_with("--") && flag.contains("-rev")
}

/// The flag naming a revision to compare against, and the revision. `None`
/// when the recipe names none — in which case the check has no baseline but
/// the registry, and nothing this workspace publishes is on one yet.
fn baseline_revision(arguments: &[String]) -> Option<(String, String)> {
    arguments
        .iter()
        .zip(arguments.iter().skip(1))
        .find(|(flag, _)| takes_a_revision(flag))
        .map(|(flag, value)| (flag.clone(), value.clone()))
}

/// `git` in this repository, stdout on success and `None` on any failure.
fn git(arguments: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(arguments)
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("running `git {}`: {e}", arguments.join(" ")));
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
}

/// A `vMAJOR.MINOR.PATCH` tag as three numbers, for ordering. `None` for a tag
/// this repo's release scheme does not produce.
fn tag_version(tag: &str) -> Option<(u64, u64, u64)> {
    let mut parts = tag.strip_prefix('v')?.split('.');
    let mut next = || parts.next()?.parse::<u64>().ok();
    let version = (next()?, next()?, next()?);
    parts.next().is_none().then_some(version)
}

/// The `v*` tags this checkout can see, newest last.
fn version_tags() -> Vec<String> {
    let mut tags: Vec<(u64, u64, u64, String)> = git(&["tag", "--list", "v*"])
        .unwrap_or_default()
        .lines()
        .filter_map(|tag| tag_version(tag).map(|(x, y, z)| (x, y, z, tag.to_owned())))
        .collect();
    tags.sort_unstable();
    tags.into_iter().map(|(_, _, _, tag)| tag).collect()
}

/// The `name` a manifest declares.
fn package_name(manifest: &str) -> Option<&str> {
    manifest_value(manifest, "package", "name").map(|name| name.trim_matches('"'))
}

/// The package names a revision's workspace holds. Read out of that revision
/// rather than out of the checkout: which crates the baseline contains is the
/// entire question an `--exclude` answers.
fn packages_at(revision: &str) -> BTreeSet<String> {
    let root = git(&["show", &format!("{revision}:Cargo.toml")])
        .unwrap_or_else(|| panic!("`{revision}` has no Cargo.toml this checkout can read"));
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
        paths.len() >= 3,
        "only {paths:?} read out of `{revision}`'s workspace members; the reader is not finding \
         the list"
    );
    paths
        .iter()
        .filter_map(|path| git(&["show", &format!("{revision}:{path}/Cargo.toml")]))
        .filter_map(|manifest| package_name(&manifest).map(str::to_owned))
        .collect()
}

#[test]
fn the_semver_gate_names_a_baseline_it_can_reach() {
    let justfile = read("Justfile");
    assert!(
        recipes_in_group(&justfile, "gate").contains(SEMVER_GATE),
        "`{SEMVER_GATE}` is not a `[group('gate')]` recipe. Everything below is about what its \
         arguments assert, and nothing asserts anything while nothing runs it."
    );

    let arguments = semver_arguments(&justfile);
    let (flag, revision) = baseline_revision(&arguments).unwrap_or_else(|| {
        panic!(
            "`just {SEMVER_GATE}` names no baseline revision: {arguments:?}\n\
             Without one cargo-semver-checks compares against the registry, and nothing this \
             workspace publishes is on the registry yet — so the gate errors rather than \
             checks. `--baseline-rev <tag>` resolves out of this repository's own history, \
             which is why the check was available long before the first publish."
        )
    });

    assert!(
        git(&[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{revision}^{{commit}}")
        ])
        .is_some(),
        "`{flag} {revision}` names a revision this checkout cannot resolve. Locally that is a \
         missing tag; in CI it is a checkout without `fetch-depth: 0`, and either way the gate \
         fails for a reason that looks like the tool being broken rather than the API having \
         moved."
    );

    let tags = version_tags();
    let newest = tags.last().unwrap_or_else(|| {
        panic!("this checkout can see no `v*` tag at all; `fetch-depth: 0` fetches them")
    });
    assert_eq!(
        &revision, newest,
        "`{flag} {revision}` is behind `{newest}`, the newest release tag. The flag is \
         scaffolding for the window before the first publish: while it is here it has to name \
         the latest release, or the gate is quietly measuring against an older surface than the \
         one anybody shipped. Once a version is on crates.io, delete the flag — the registry \
         version is cargo-semver-checks' own default baseline and there is no literal left to \
         keep in step."
    );
}

#[test]
fn every_package_the_semver_gate_excludes_is_one_its_baseline_does_not_hold() {
    let justfile = read("Justfile");
    let arguments = semver_arguments(&justfile);
    let (_, revision) = baseline_revision(&arguments)
        .unwrap_or_else(|| panic!("`just {SEMVER_GATE}` names no baseline revision"));
    let baseline = packages_at(&revision);
    assert!(
        baseline.contains("aozora-flavored-markdown"),
        "`{revision}` came out holding {baseline:?}; the reader is not finding its members"
    );

    let excluded: BTreeSet<&str> = flag_values(&arguments, "--exclude").into_iter().collect();
    let still_there: Vec<&&str> = excluded
        .iter()
        .filter(|package| baseline.contains(**package))
        .collect();
    assert!(
        still_there.is_empty(),
        "the gate excludes {still_there:?}, and `{revision}` holds them. An exclusion is a \
         statement that the baseline has nothing to compare against — the two epub crates \
         joined this workspace after that tag (ADR-0018) and stop the run outright. A crate the \
         baseline does hold is one this gate could check and has been told not to, which is how \
         an exclusion list becomes the place breaking changes go to be unseen."
    );

    let published: BTreeSet<String> = workspace_members()
        .iter()
        .filter(|member| is_published(&member.manifest))
        .filter_map(|member| package_name(&member.manifest).map(str::to_owned))
        .collect();
    assert!(
        published.len() >= 2,
        "only {published:?} read as published; the reader is not finding the manifests"
    );
    let unchecked: Vec<&String> = published
        .iter()
        .filter(|package| baseline.contains(*package) && excluded.contains(package.as_str()))
        .collect();
    assert!(
        unchecked.is_empty(),
        "these reach crates.io, are in the baseline, and are excluded from the only gate that \
         compares their public API against it: {unchecked:?}"
    );
}

// ---------------------------------------------------------------------------
// what the arguments do when handed a break
// ---------------------------------------------------------------------------

/// The probe's clean public surface, and the same surface with one public
/// field removed — a break every version of cargo-semver-checks reports, under
/// its own `struct_pub_field_missing` lint.
const PROBE_INTACT: &str = "pub struct Kept {\n    pub field: u8,\n}\n";
const PROBE_BROKEN: &str = "pub struct Kept {}\n";

/// The tag the probe repository carries on its intact revision.
const PROBE_BASELINE: &str = "v0.1.0";

/// A throwaway crate in a git repository of its own, tagged at an intact
/// public surface. Rewriting it and asking the gate's own arguments about it
/// is the only way to know those arguments report anything: every other rule
/// here would pass just as well on a flag that is a typo.
struct SemverProbe {
    root: PathBuf,
    dir: PathBuf,
}

impl SemverProbe {
    fn new() -> Self {
        // The build directory is a SIBLING of the crate, not the `target/`
        // inside it. cargo-semver-checks clones the baseline revision into
        // the build directory, and a clone under the project root is a second
        // manifest for the same package name — cargo refuses the whole run as
        // ambiguous before either surface is read.
        let root = scratch("semver-probe");
        let probe = Self {
            dir: root.join("crate"),
            root,
        };
        probe.write("0.1.0", PROBE_INTACT);
        probe.git(&["init", "--quiet"]);
        probe.git(&["add", "--all"]);
        probe.git(&[
            "-c",
            "user.name=probe",
            "-c",
            "user.email=probe@example.invalid",
            "-c",
            "commit.gpgsign=false",
            "commit",
            "--quiet",
            "--message=probe",
        ]);
        probe.git(&["tag", PROBE_BASELINE]);
        probe
    }

    fn write(&self, version: &str, source: &str) {
        let src = self.dir.join("src");
        fs::create_dir_all(&src).unwrap_or_else(|e| panic!("creating {}: {e}", src.display()));
        let manifest = format!(
            "[workspace]\n\n\
             [package]\n\
             name = \"aozora_md_semver_probe\"\n\
             version = \"{version}\"\n\
             edition = \"2021\"\n\n\
             [lib]\n\
             path = \"src/lib.rs\"\n"
        );
        fs::write(self.dir.join("Cargo.toml"), manifest)
            .unwrap_or_else(|e| panic!("writing the probe manifest: {e}"));
        fs::write(src.join("lib.rs"), source)
            .unwrap_or_else(|e| panic!("writing the probe source: {e}"));
    }

    fn git(&self, arguments: &[&str]) {
        let out = Command::new("git")
            .args(arguments)
            .current_dir(&self.dir)
            .output()
            .unwrap_or_else(|e| panic!("running `git {}`: {e}", arguments.join(" ")));
        assert!(
            out.status.success(),
            "the probe repository could not be built (`git {}`):\n{}",
            arguments.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }

    /// Does the gate's own sub-command and baseline flag accept this version
    /// of this source, compared against the intact tag?
    fn accepts(&self, call: &SemverCall, version: &str, source: &str) -> bool {
        self.write(version, source);
        let out = Command::new("cargo")
            .args([
                "semver-checks",
                &call.subcommand,
                &call.baseline_flag,
                PROBE_BASELINE,
            ])
            .current_dir(&self.dir)
            .env("CARGO_TARGET_DIR", self.root.join("build"))
            .output()
            .unwrap_or_else(|e| {
                panic!(
                    "running cargo semver-checks: {e}\n\
                     This suite runs inside the dev image (ADR-0002), where it is installed."
                )
            });
        let accepted = out.status.success();
        let report = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            report.contains("Summary"),
            "cargo semver-checks did not reach a verdict on the probe:\n{report}"
        );
        accepted
    }
}

impl Drop for SemverProbe {
    fn drop(&mut self) {
        drop(fs::remove_dir_all(&self.root));
    }
}

/// How the gate calls cargo-semver-checks, for a probe to be asked the same
/// way. `check-release` is what compares two surfaces; a bare `check` would be
/// a different question, and reading the recipe rather than spelling the
/// arguments here is what makes the probe measure this gate.
struct SemverCall {
    subcommand: String,
    baseline_flag: String,
}

impl SemverCall {
    fn of(justfile: &str) -> Self {
        let arguments = semver_arguments(justfile);
        let subcommand = arguments
            .iter()
            .find(|token| !token.starts_with('-'))
            .unwrap_or_else(|| {
                panic!("`just {SEMVER_GATE}` hands cargo-semver-checks no sub-command")
            })
            .clone();
        let (baseline_flag, _) = baseline_revision(&arguments)
            .unwrap_or_else(|| panic!("`just {SEMVER_GATE}` names no baseline revision"));
        Self {
            subcommand,
            baseline_flag,
        }
    }
}

#[test]
fn the_arguments_the_semver_gate_passes_are_what_makes_a_breaking_change_fail() {
    let justfile = read("Justfile");
    let call = SemverCall::of(&justfile);
    let probe = SemverProbe::new();

    // The control, and the reason the second half means anything: at a patch
    // bump over an unchanged surface these arguments produce a pass. A gate
    // that failed here would fail on everything, which is a different bug
    // wearing this one's clothes.
    assert!(
        probe.accepts(&call, "0.1.1", PROBE_INTACT),
        "`{} {}` rejects a patch release that changed nothing",
        call.subcommand,
        call.baseline_flag
    );

    assert!(
        !probe.accepts(&call, "0.1.1", PROBE_BROKEN),
        "`{} {}` passes a patch release that removed a public field. The recipe is wired, the \
         leg is green, and the answer is worth nothing — which is the state this gate was in \
         for the whole of the public-surface rebuild, for a different reason.",
        call.subcommand,
        call.baseline_flag
    );
}

#[test]
fn the_semver_gate_answers_vacuously_while_this_workspace_outruns_its_baseline() {
    // A debt, pinned rather than described — which is why neither the recipe
    // nor ADR-0015 describes it any more. cargo reads `0.4` -> `0.5` as a
    // major bump, and a major bump permits every break there is: the same
    // removal the test above catches is skipped here, and the run reports
    // `no semver update required` and exits 0.
    //
    // That is the correct answer for this cycle — the breaking changes are the
    // plan and 0.5.0 declares them — and it means the green leg on every PR
    // until the next release asserts only that the declared version covers
    // what changed. The gate starts biting the moment the baseline is a
    // version this workspace is merely a patch ahead of, and that is the
    // moment this test fails and wants deleting.
    let justfile = read("Justfile");
    let probe = SemverProbe::new();
    assert!(
        probe.accepts(&SemverCall::of(&justfile), "0.2.0", PROBE_BROKEN),
        "a 0.y major bump no longer absorbs a removed public field. cargo-semver-checks has \
         changed what it skips, and this test recorded the opposite."
    );

    let arguments = semver_arguments(&justfile);
    let (_, revision) = baseline_revision(&arguments)
        .unwrap_or_else(|| panic!("`just {SEMVER_GATE}` names no baseline revision"));
    let baseline = tag_version(&revision)
        .unwrap_or_else(|| panic!("`{revision}` is not a vMAJOR.MINOR.PATCH tag"));
    let manifest = read("Cargo.toml");
    let declared = manifest_value(&manifest, "workspace.package", "version").map_or_else(
        || panic!("the workspace manifest declares no version"),
        |version| format!("v{}", version.trim_matches('"')),
    );
    let declared = tag_version(&declared)
        .unwrap_or_else(|| panic!("`{declared}` is not a MAJOR.MINOR.PATCH version"));

    // cargo's 0.y rule: below 1.0 the minor is the breaking-change axis.
    let breaking = if baseline.0 == 0 && declared.0 == 0 {
        declared.1 > baseline.1
    } else {
        declared.0 > baseline.0
    };
    assert!(
        breaking,
        "this workspace declares {declared:?} against a {baseline:?} baseline, so the gate is \
         no longer skipping its lints — it is comparing public API for real. Good: delete this \
         test, because the pass it pins has stopped being vacuous."
    );
}

/// The target directory a command line points cargo at, as written. The value
/// is a shell expansion and this reader does not expand it: the question is
/// whether two gates aim cargo at the same directory, not what that directory
/// is called on any one machine.
fn target_directory_on(line: &str) -> Option<&str> {
    line.split_once("CARGO_TARGET_DIR=")?
        .1
        .split_whitespace()
        .next()
}

/// Where a recipe sends cargo's build output. `None` is not "nowhere" — it is
/// the ambient directory the dev image bakes in, which is the one every gate
/// that names none shares.
fn target_directory_of(justfile: &str, recipe: &str) -> Option<String> {
    joined_body(justfile, recipe)
        .iter()
        .find_map(|line| target_directory_on(line).map(str::to_owned))
}

#[test]
fn the_semver_gate_does_not_build_its_rustdoc_where_the_doc_gates_build_theirs() {
    // Both sides of this gate are rustdoc JSON for a crate called
    // `aozora_flavored_markdown` — the current one and the baseline tag's — and
    // rustdoc names its output file after the crate, not after the package it
    // was built from. So both sides resolve to one `doc/<crate>.json` under
    // whatever target directory cargo is aimed at: the same output-path clash
    // `_NO_COLLISION` catches within a single `cargo doc` pass, one directory
    // up and across two gates.
    //
    // Sharing it does not cost time, it costs the answer. Measured on the
    // arrangement this test was written for: `just doc` first, then this gate,
    // and cargo reads the crate's doc unit as fresh, skips the current-side
    // build, and leaves the BASELINE's JSON where the current one belongs — so
    // the gate compares the baseline against itself, reads the version as
    // unchanged, drops out of "major bump, skip everything" into per-lint
    // checking, and fails on a feature the released version has and this one
    // dropped. That is `just ci`'s own order, and the failure it prints names a
    // break in a workspace that has none.
    //
    // `cargo semver-checks` takes no `--target-dir`, so the environment is the
    // only place to say this, and an env assignment is deletable in a way a
    // flag is not. Hence a test.
    let justfile = read("Justfile");
    let semver_directory = target_directory_of(&justfile, SEMVER_GATE).unwrap_or_else(|| {
        panic!(
            "`just {SEMVER_GATE}` names no CARGO_TARGET_DIR, so it builds both of its rustdoc \
             JSONs where the doc gates build theirs. After a `cargo doc` over the same \
             directory this gate stops comparing the current surface at all and reports the \
             baseline's own features as removed."
        )
    });

    // Naming the ambient directory back to itself would satisfy the read above
    // and change nothing, so the value has to go somewhere below it.
    assert!(
        semver_directory
            .rsplit_once('}')
            .is_none_or(|(_, tail)| tail.trim_matches('"').contains('/')),
        "`just {SEMVER_GATE}` points cargo at `{semver_directory}`, which is the directory the \
         doc gates already build in under another spelling"
    );

    let doc_gates: Vec<String> = recipes_in_group(&justfile, "gate")
        .into_iter()
        .filter(|recipe| {
            joined_body(&justfile, recipe)
                .iter()
                .any(|line| builds_rustdoc(line))
        })
        .collect();
    assert!(
        doc_gates.len() >= 2,
        "only {doc_gates:?} read as rustdoc-building gates; the reader is not finding them, so \
         what follows compares this gate against nothing"
    );

    for gate in &doc_gates {
        assert_ne!(
            target_directory_of(&justfile, gate),
            Some(semver_directory.clone()),
            "`just {gate}` and `just {SEMVER_GATE}` aim cargo at one target directory, and both \
             write rustdoc output for the same crate name into it"
        );
    }
}

// ---------------------------------------------------------------------------
// the history a recipe needs and a checkout does not fetch
// ---------------------------------------------------------------------------
//
// `actions/checkout` clones one commit. Everything else in this repo is happy
// with that, which is why two gates that are not had to be found by hand: the
// commit range `just commitlint` reads and the tag `just semver` resolves are
// both in history a depth-1 clone does not carry. Getting it wrong fails in
// the worst shape a gate has — the tool reports a missing revision, so the run
// reads as broken tooling rather than as the thing the gate exists to say.

/// The shape a revision range is written in. `origin/main..HEAD` sits in a
/// recipe *header* as a parameter default, so the header is read too.
const REVISION_RANGE: &str = "..";

/// Does this token name a git revision range rather than a path? `../..` is
/// the thing that is not one.
fn names_a_revision(token: &str) -> bool {
    token.contains(REVISION_RANGE) && !token.starts_with('.') && !token.contains("./")
}

/// The recipes that name a git revision, and therefore need the history that
/// revision lives in.
fn recipes_needing_history(justfile: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for recipe in justfile
        .lines()
        .filter(|line| !line.starts_with([' ', '\t', '#', '[']))
        .filter_map(recipe_name)
    {
        let header = recipe_header(justfile, &recipe);
        let mut tokens: Vec<String> = shell_tokens(strip_comment(header));
        tokens.extend(
            joined_body(justfile, &recipe)
                .iter()
                .flat_map(|line| shell_tokens(line)),
        );
        let names = tokens.iter().any(|token| names_a_revision(token))
            || tokens.iter().any(|token| takes_a_revision(token));
        if names {
            out.insert(recipe);
        }
    }
    out
}

/// The recipes a workflow job can run. A job whose matrix expands the gate
/// manifest runs every containerized gate, one leg at a time, and names none
/// of them — which is the whole point of deriving the legs and the reason a
/// reader of `run:` lines alone would see that job as running nothing.
fn recipes_a_job_can_run(justfile: &str, workflow: &str, job: &str) -> BTreeSet<String> {
    let Some(lines) = job_lines(workflow, job) else {
        return BTreeSet::new();
    };
    let mut out = recipes_invoked_in(&lines);
    if lines
        .iter()
        .any(|line| strip_comment(line).contains("fromJSON(needs.gates.outputs."))
    {
        let native = recipes_in_group(justfile, "native");
        out.extend(
            recipes_in_group(justfile, "gate")
                .into_iter()
                .filter(|gate| !native.contains(gate)),
        );
    }
    out
}

/// Does this job check out the whole history rather than the tip?
fn checks_out_full_history(lines: &[&str]) -> bool {
    lines
        .iter()
        .any(|line| strip_comment(line).trim() == "fetch-depth: 0")
}

#[test]
fn every_job_that_runs_a_recipe_naming_a_revision_checks_out_the_history_it_needs() {
    let justfile = read("Justfile");
    let needs_history = recipes_needing_history(&justfile);
    assert_eq!(
        needs_history,
        [SEMVER_GATE.to_owned(), "commitlint".to_owned()]
            .into_iter()
            .collect::<BTreeSet<String>>(),
        "the recipes naming a git revision are no longer the two this rule was measured \
         against. A new one is a new job to check; one gone is a reader that has stopped \
         finding them."
    );

    let mut shallow = Vec::new();
    let mut asked = 0;
    for path in workflow_files() {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        for job in job_keys(&text) {
            let running: BTreeSet<String> = recipes_a_job_can_run(&justfile, &text, &job)
                .intersection(&needs_history)
                .cloned()
                .collect();
            if running.is_empty() {
                continue;
            }
            asked += 1;
            let lines = job_lines(&text, &job).unwrap_or_default();
            if !checks_out_full_history(&lines) {
                shallow.push(format!("  {label}'s `{job}` job runs {running:?}"));
            }
        }
    }
    // The defect before the floor: a shallow job is a fact about this repo,
    // and the count below is only a fact about the reader.
    assert!(
        shallow.is_empty(),
        "jobs that resolve a revision out of a depth-1 clone:\n{}\n\
         `actions/checkout` fetches one commit unless told otherwise. The recipe then fails on \
         a revision it cannot find, which looks like the check being broken and not like the \
         answer it was asked for. Add `fetch-depth: 0` to that job's checkout.",
        shallow.join("\n")
    );
    assert!(
        asked >= 3,
        "only {asked} job(s) came out running a recipe that needs history; the reader is not \
         finding them. Three is what this repo has: the gate matrix and the commit-range job in \
         ci.yml, and the publish preflight."
    );
}

// ---------------------------------------------------------------------------
// the ladder that reaches crates.io
// ---------------------------------------------------------------------------
//
// `publish = false` is this workspace's one declaration of what leaves it for
// crates.io, and three rules above already derive their subject from it: the
// docs.rs build every published crate asks for, the coverage denominator no
// published source may fall out of, and the binaries a test has to run as a
// process. The consumer that never derived it is the workflow that does the
// uploading. It spelled its crates as two literals in a shell `for` while the
// manifests said four, so the EPUB pair consolidated in by ADR-0018 was
// unreachable from every release path this repo has — and none of the three
// readers above was pointed anywhere that could say so. A fourth reading of
// one declaration, this time by the file that acts on it.
//
// The rule is over the SET and not over the spelling, because the spelling is
// what was in question: `cargo publish --workspace` answers it by
// construction, an explicit `-p` ladder answers it by listing, and a selection
// assembled at run time out of a shell variable does not answer it at all.
// That last state is not "unknown, therefore fine" — an upload nobody can
// predict by reading the file is the defect itself, not a gap in the reader.

/// The workflow that uploads to crates.io.
const PUBLISH_WORKFLOW: &str = ".github/workflows/publish-crates.yml";

/// The crates.io name of a member. The directory is not it: nothing makes a
/// crate's name match the directory it sits in, and the name is what a `-p`
/// selects and what crates.io serves.
fn member_name(member: &Member) -> String {
    package_name(&member.manifest)
        .unwrap_or_else(|| {
            panic!(
                "{}/Cargo.toml declares no `[package] name`; the reader cannot say what this \
                 member is called on crates.io",
                member.path
            )
        })
        .to_owned()
}

/// Every crate this repo uploads to crates.io, by name, off the manifests.
fn publishable_crates() -> BTreeSet<String> {
    workspace_members()
        .iter()
        .filter(|member| is_published(&member.manifest))
        .map(member_name)
        .collect()
}

/// What one `cargo publish` selects, as that command spells it.
#[derive(Debug, PartialEq, Eq)]
enum Selection {
    /// `--workspace`, less whatever `--exclude` drops. The set is a fact about
    /// the manifests, so cargo answers it and this reader defers.
    Workspace(BTreeSet<String>),
    /// `-p` / `--package`, spelled out here.
    Named(BTreeSet<String>),
    /// The command names no set this file can resolve.
    Opaque(String),
}

/// A crate name as an argument may spell it. A `$` or a `${…}` left over from
/// a shell expansion is not one, which is the whole point of asking. Nor is a
/// glob: `-p 'aozora-*'` is a set cargo resolves at run time, and a set this
/// file cannot resolve is the state the rules below exist to reject.
fn crate_name_word(token: &str) -> Option<String> {
    let shaped = !token.is_empty()
        && token.starts_with(|ch: char| ch.is_ascii_alphanumeric())
        && token
            .chars()
            .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
    shaped.then(|| token.to_owned())
}

/// Where `cargo publish` starts in a token stream, skipping a `+toolchain`.
fn publish_at(tokens: &[String]) -> Option<usize> {
    (0..tokens.len()).find(|&at| {
        if tokens[at] != "cargo" {
            return false;
        }
        let mut next = at + 1;
        while tokens.get(next).is_some_and(|token| token.starts_with('+')) {
            next += 1;
        }
        tokens.get(next).map(String::as_str) == Some("publish")
    })
}

/// The three sinks a package selector can land in, so one loop can fill them.
#[derive(Default)]
struct Selectors {
    named: BTreeSet<String>,
    excluded: BTreeSet<String>,
    opaque: Vec<String>,
}

impl Selectors {
    /// One `-p` / `--exclude` value, which cargo lets carry a comma list.
    fn take(&mut self, exclude: bool, raw: &str) {
        for part in raw.split(',').filter(|part| !part.trim().is_empty()) {
            match (crate_name_word(part), exclude) {
                (Some(name), false) => {
                    self.named.insert(name);
                }
                (Some(name), true) => {
                    self.excluded.insert(name);
                }
                (None, _) => self
                    .opaque
                    .push(format!("`{part}` is not a crate name this file spells out")),
            }
        }
    }
}

fn publish_selection(tokens: &[String]) -> Selection {
    let mut found = Selectors::default();
    let mut workspace = false;
    let mut pending: Option<bool> = None;
    for token in tokens {
        if let Some(exclude) = pending.take() {
            found.take(exclude, token);
            continue;
        }
        let (flag, inline) = token
            .split_once('=')
            .map_or((token.as_str(), None), |(flag, value)| (flag, Some(value)));
        match flag {
            "--workspace" | "--all" => workspace = true,
            "-p" | "--package" | "--exclude" => match inline {
                Some(value) => found.take(flag == "--exclude", value),
                None => pending = Some(flag == "--exclude"),
            },
            _ => {}
        }
    }
    if pending.is_some() {
        found
            .opaque
            .push("a package flag with nothing after it".to_owned());
    }
    if !found.opaque.is_empty() {
        return Selection::Opaque(found.opaque.join("; "));
    }
    if workspace {
        return Selection::Workspace(found.excluded);
    }
    if found.named.is_empty() {
        return Selection::Opaque(
            "neither `--workspace` nor a `-p`, so cargo publishes whichever single crate the \
             working directory happens to hold"
                .to_owned(),
        );
    }
    Selection::Named(found.named)
}

/// One `cargo publish` this repo runs.
struct PublishCall {
    /// Where it was found: a workflow job, or a `Justfile` recipe.
    site: String,
    /// The line as it is written, and behind a `→` the recipe line it reaches.
    command: String,
    /// The recipe the site named, when it reached the command through one.
    via: Option<String>,
    dry_run: bool,
    /// The tokens from `cargo` onward, so a rule can ask about a flag.
    arguments: Vec<String>,
    selection: Selection,
}

impl PublishCall {
    /// Read one call off the tokens that start at its `cargo`.
    fn read(site: String, command: String, via: Option<String>, tokens: &[String]) -> Self {
        Self {
            site,
            command,
            via,
            dry_run: tokens.iter().any(|token| token == "--dry-run"),
            arguments: tokens.to_vec(),
            selection: publish_selection(tokens),
        }
    }

    fn has(&self, flag: &str) -> bool {
        self.arguments.iter().any(|token| token == flag)
    }
}

/// Every command line a recipe runs, its dependencies included and `{{VAR}}`
/// resolved: the walk [`commands_owned_by_gates`] makes over every gate, asked
/// of one recipe.
fn recipe_commands(justfile: &str, recipe: &str) -> Vec<String> {
    let variables = plain_variables(justfile);
    let mut pending = vec![recipe.to_owned()];
    let mut seen = BTreeSet::new();
    let mut out = Vec::new();
    while let Some(name) = pending.pop() {
        if !seen.insert(name.clone()) || !recipe_exists(justfile, &name) {
            continue;
        }
        pending.extend(header_dependencies(recipe_header(justfile, &name)));
        for line in recipe_body(justfile, &name) {
            out.push(expand(strip_comment(line), &variables));
        }
    }
    out
}

/// Every `cargo publish` a workflow drives, the ones it reaches through a
/// recipe included.
///
/// Following the `just` is not a convenience. The dry run is a
/// `[group('gate')]` recipe (DEV-224), so the workflow calls it instead of
/// keeping a second copy of the command — and a reader that stopped at the
/// `just` would find no preflight in this file at all, and report the
/// arrangement that put packaging in front of every merge as the one that
/// packages nothing.
fn publish_calls(workflow: &str, justfile: &str) -> Vec<PublishCall> {
    let mut out = Vec::new();
    for job in job_keys(workflow) {
        for line in job_lines(workflow, &job).unwrap_or_default() {
            let written = strip_comment(line).trim().to_owned();
            let mut bodies = vec![(None, written.clone())];
            for recipe in recipes_invoked_in(&[line]) {
                bodies.extend(
                    recipe_commands(justfile, &recipe)
                        .into_iter()
                        .map(|body| (Some(recipe.clone()), body)),
                );
            }
            for (via, body) in bodies {
                let tokens = shell_tokens(&body);
                let Some(at) = publish_at(&tokens) else {
                    continue;
                };
                let command = match &via {
                    Some(_) => format!("{written}  →  {}", body.trim()),
                    None => written.clone(),
                };
                out.push(PublishCall::read(job.clone(), command, via, &tokens[at..]));
            }
        }
    }
    out
}

/// Every `cargo publish` a `[group('gate')]` recipe runs — the packaging a
/// pull request does, read off the same file the gate manifest is read off.
fn publish_calls_from_gates(justfile: &str) -> Vec<PublishCall> {
    let mut out = Vec::new();
    for gate in recipes_in_group(justfile, "gate") {
        for body in recipe_commands(justfile, &gate) {
            let tokens = shell_tokens(&body);
            let Some(at) = publish_at(&tokens) else {
                continue;
            };
            out.push(PublishCall::read(
                format!("just {gate}"),
                body.trim().to_owned(),
                Some(gate.clone()),
                &tokens[at..],
            ));
        }
    }
    out
}

/// The crates a set of calls reaches, and the calls whose reach is unreadable.
/// `--workspace` is resolved against the manifests, which is the same answer
/// cargo derives and the reason that spelling cannot fall behind them.
fn crates_reached(
    calls: &[&PublishCall],
    publishable: &BTreeSet<String>,
) -> (BTreeSet<String>, Vec<String>) {
    let mut reached = BTreeSet::new();
    let mut opaque = Vec::new();
    for call in calls {
        match &call.selection {
            Selection::Workspace(excluded) => {
                reached.extend(publishable.difference(excluded).cloned());
            }
            Selection::Named(named) => reached.extend(named.iter().cloned()),
            Selection::Opaque(why) => {
                opaque.push(format!("  {}: {why}\n      {}", call.site, call.command));
            }
        }
    }
    (reached, opaque)
}

/// The message an unreadable selection fails with, spelled once because both
/// rules below reject the same state for the same reason.
fn unreadable(opaque: &[String]) -> String {
    format!(
        "a `cargo publish` whose crates cannot be read off the file:\n{}\n\
         The manifests are the one statement of what reaches crates.io — `publish = false` is \
         the opt-out and cargo honours it — and this workflow is the only thing that acts on \
         that statement. A selection assembled at run time out of a shell variable means \
         nothing here can compare the two, so the set can fall behind the manifests exactly as \
         it did when the EPUB pair arrived. Spell it `--workspace` and let cargo derive the \
         set, or list the crates with `-p`.",
        opaque.join("\n")
    )
}

#[test]
fn every_crate_this_repo_publishes_is_one_the_publish_workflow_uploads() {
    let members = workspace_members();
    let publishable = publishable_crates();
    // The same blindness check the docs.rs rule makes, for the same reason:
    // a reader that cannot tell `publish = false` from its absence answers
    // every question below with the whole workspace or with nothing.
    assert!(
        publishable.len() >= 2 && publishable.len() < members.len(),
        "{} of {} members read as published; the reader is not telling `publish = false` apart \
         from its absence",
        publishable.len(),
        members.len()
    );

    let workflow = read(PUBLISH_WORKFLOW);
    let calls = publish_calls(&workflow, &read("Justfile"));
    let uploads: Vec<&PublishCall> = calls.iter().filter(|call| !call.dry_run).collect();
    assert!(
        !uploads.is_empty(),
        "{PUBLISH_WORKFLOW} came out running no `cargo publish` that uploads anything. Either \
         the release path has moved and this rule points at nothing, or the reader has stopped \
         finding it — and a reader finding nothing calls every ladder complete."
    );

    let (uploaded, opaque) = crates_reached(&uploads, &publishable);
    assert!(opaque.is_empty(), "{}", unreadable(&opaque));
    assert_eq!(
        uploaded,
        publishable,
        "the crates {PUBLISH_WORKFLOW} uploads are not the crates this workspace publishes.\n  \
         never uploaded: {:?}\n  uploaded but not a publishable member: {:?}\n\
         A crate whose manifest says it goes to crates.io and that no release path names is a \
         crate nobody can `cargo add`, which is the problem ADR-0015 exists to solve.",
        publishable.difference(&uploaded).collect::<Vec<_>>(),
        uploaded.difference(&publishable).collect::<Vec<_>>()
    );
}

#[test]
fn the_publish_preflight_verifies_every_crate_the_live_step_uploads() {
    // The preflight's whole claim is that a packaging regression surfaces
    // "before anything is uploaded", and an upload is the one operation
    // crates.io will not let anybody take back. That claim is about a SET, and
    // the job was dry-running one crate in front of a ladder that uploaded
    // two — so the rung most likely to break was the rung it never packaged.
    let publishable = publishable_crates();
    let workflow = read(PUBLISH_WORKFLOW);
    let calls = publish_calls(&workflow, &read("Justfile"));

    let packaged: Vec<&PublishCall> = calls.iter().filter(|call| call.dry_run).collect();
    let uploading: Vec<&PublishCall> = calls.iter().filter(|call| !call.dry_run).collect();
    let (verified, dry_opaque) = crates_reached(&packaged, &publishable);
    let (uploaded, live_opaque) = crates_reached(&uploading, &publishable);
    let opaque: Vec<String> = dry_opaque.into_iter().chain(live_opaque).collect();
    assert!(opaque.is_empty(), "{}", unreadable(&opaque));
    assert!(
        !verified.is_empty() && !uploaded.is_empty(),
        "{PUBLISH_WORKFLOW} came out with {} crate(s) dry-run and {} uploaded; a rule about one \
         set covering another says nothing over an empty one",
        verified.len(),
        uploaded.len()
    );

    let unverified: Vec<&String> = uploaded.difference(&verified).collect();
    assert!(
        unverified.is_empty(),
        "{PUBLISH_WORKFLOW} uploads {unverified:?} without packaging them first. The preflight \
         is the only check standing between a broken manifest and an upload nobody can \
         retract, and it is only that for the crates it actually packages."
    );

    // A preflight that covers the set and does not run first covers nothing.
    let upload_jobs: BTreeSet<&str> = uploading.iter().map(|call| call.site.as_str()).collect();
    let packaging: BTreeSet<&str> = packaged.iter().map(|call| call.site.as_str()).collect();
    for job in upload_jobs {
        let waits_on = job_needs(&workflow, job);
        assert!(
            packaging.iter().any(|first| waits_on.contains(*first)),
            "{PUBLISH_WORKFLOW}'s `{job}` job uploads to crates.io without `needs:` on a job \
             that dry-runs first (it waits on {waits_on:?}, the preflight is {packaging:?}). \
             Two jobs with no edge between them run at once."
        );
    }
}

#[test]
fn the_packaging_the_release_path_relies_on_is_a_gate_every_merge_runs() {
    // The rule above compares two sets inside one file, and every answer it
    // can give is about that file. `publish-crates.yml` runs on
    // `workflow_dispatch` alone, so a preflight that covers the whole ladder
    // there still first speaks on the day somebody decides to release — which
    // is the day the packaging question is worth the least, because by then
    // the tree is a tag and the change that broke it merged weeks ago. That
    // was the state of this repo until DEV-224: `cargo publish --dry-run` was
    // written out in that workflow and nowhere else, so no pull request has
    // ever built these crates the way a consumer receives them, and while
    // `comrak` was a path dependency (ADR-0024) the graph a consumer resolves
    // had never been built on `main` at all.
    //
    // So this rule asks the Justfile, not the workflow: is the packaging a
    // GATE. The gate manifest carries the rest — `[group('gate')]` is what
    // `just ci` asserts its lanes against and what ci.yml expands its matrix
    // from, both already rules of their own — so naming the group here is
    // naming every path a gate runs on.
    let justfile = read("Justfile");
    let publishable = publishable_crates();
    let packaging = publish_calls_from_gates(&justfile);
    assert!(
        !packaging.is_empty(),
        "no `[group('gate')]` recipe runs a `cargo publish`, so nothing before a merge builds \
         the four crates as a consumer receives them. Every other compile gate builds the \
         WORKSPACE — one target directory, path dependencies resolved in place, every file on \
         disk reachable from every crate — and none of them can see a file the build reads and \
         the package does not carry, or a rung that only builds because the rung under it came \
         out of the workspace instead of a registry."
    );

    for call in &packaging {
        assert!(
            call.dry_run,
            "`{}` runs a `cargo publish` that is not a dry run:\n      {}\nA gate is something \
             anyone can run, and an upload is the one operation crates.io will not let anybody \
             take back.",
            call.site, call.command
        );
        // `--no-verify` is the flag that would leave this gate green while
        // measuring nothing: cargo still writes the tarball and skips the
        // build of it, which is the entire question being asked here.
        assert!(
            !call.has("--no-verify"),
            "`{}` packages without building what it packaged:\n      {}\n`--no-verify` skips the \
             verify build, so the gate would answer \"the files can be collected\" to a question \
             about whether they compile.",
            call.site,
            call.command
        );
    }

    let calls: Vec<&PublishCall> = packaging.iter().collect();
    let (packaged, opaque) = crates_reached(&calls, &publishable);
    assert!(opaque.is_empty(), "{}", unreadable(&opaque));
    assert_eq!(
        packaged,
        publishable,
        "the crates a gate packages are not the crates this workspace publishes.\n  \
         never packaged: {:?}\n  packaged but not a publishable member: {:?}\n\
         A rung nothing packages is a rung whose first build as a package happens during the \
         release.",
        publishable.difference(&packaged).collect::<Vec<_>>(),
        packaged.difference(&publishable).collect::<Vec<_>>()
    );

    // And the release path reaches that same recipe rather than keeping a
    // copy: the preflight is the gate, so a change to what packaging means
    // cannot reach one of the two and not the other.
    let workflow = read(PUBLISH_WORKFLOW);
    let gates = recipes_in_group(&justfile, "gate");
    let mut preflights = 0;
    for call in publish_calls(&workflow, &justfile)
        .iter()
        .filter(|call| call.dry_run)
    {
        preflights += 1;
        let via = call.via.as_deref().unwrap_or("");
        assert!(
            gates.contains(via),
            "{PUBLISH_WORKFLOW}'s `{}` job writes its own dry run:\n      {}\nThe packaging gate \
             exists; this step is a second copy of it, and the copy that drifts is whichever one \
             nobody is reading.",
            call.site,
            call.command
        );
    }
    assert!(
        preflights > 0,
        "{PUBLISH_WORKFLOW} came out with no dry run at all, so the release path packages \
         nothing before it uploads"
    );
}

#[test]
fn the_upload_still_answers_to_git_though_the_gate_does_not() {
    // `--allow-dirty` is the one thing the gate asks for that a release must
    // not: it suppresses "is every file in this package committed", which is
    // not the packaging gate's question — `just ci` runs BEFORE the commit
    // exists, and a gate that declines to answer there is a gate that only
    // ever runs on a runner — but IS the upload's, because the tarball
    // crates.io serves carries a `.cargo_vcs_info.json` naming the revision it
    // claims to come from. The flag is the seam between the two, so the seam
    // is what gets asserted rather than left to the recipe's comment.
    let justfile = read("Justfile");
    let workflow = read(PUBLISH_WORKFLOW);
    let uploads: Vec<PublishCall> = publish_calls(&workflow, &justfile)
        .into_iter()
        .filter(|call| !call.dry_run)
        .collect();
    assert!(
        !uploads.is_empty(),
        "{PUBLISH_WORKFLOW} runs no upload, so this rule is measuring nothing"
    );
    for call in &uploads {
        for flag in ["--allow-dirty", "--no-verify"] {
            assert!(
                !call.has(flag),
                "{PUBLISH_WORKFLOW}'s `{}` job uploads with `{flag}`:\n      {}\nThat flag \
                 belongs to the packaging gate, where the tree is a working tree. On the upload \
                 it publishes a tarball that answers to no commit.",
                call.site,
                call.command
            );
        }
    }
}

/// Shell words that cut text out of a file rather than asking a tool for it.
const TEXT_EXTRACTORS: &[&str] = &["grep", "sed", "awk", "cut"];

/// Does this line read a version out of a Cargo manifest as text?
fn reads_a_manifest_version(line: &str) -> bool {
    let body = strip_comment(line);
    if !body.contains("Cargo.toml") || !body.contains("version") {
        return false;
    }
    shell_tokens(body)
        .iter()
        .any(|token| TEXT_EXTRACTORS.contains(&token.as_str()))
}

#[test]
fn no_workflow_reads_a_crate_version_out_of_a_manifest_by_hand() {
    // The version half of the same defect, and the half a `-p` ladder would
    // carry forward. `grep -m1 '^version' Cargo.toml` answers with the
    // workspace's version, which is an answer only while every publishable
    // crate is on the workspace line. Two of the four are not: the EPUB pair
    // is deliberately on its own 0.1.x line (ADR-0018), so that one grep names
    // a version those crates do not have — and it was being used to decide
    // whether they were already on crates.io.
    //
    // Scope is the workflows, which are where this repo names a version to a
    // registry. The `Justfile`'s `verify-version-pins` greps manifests too and
    // is not in scope: what it reads there is a dependency REQUIREMENT, and
    // comparing two declarations of one requirement is exactly the right way
    // to ask whether they agree.
    let mut hand_read = Vec::new();
    let mut scanned = 0_usize;
    for path in workflow_files() {
        let label = label_of(&path);
        let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {label}: {e}"));
        scanned += 1;
        for (index, line) in text.lines().enumerate() {
            if reads_a_manifest_version(line) {
                hand_read.push(format!("  {label}:{}: {}", index + 1, line.trim()));
            }
        }
    }
    assert!(
        scanned >= 5,
        "only {scanned} workflow(s) read; the scan is not finding the files it is written over"
    );
    assert!(
        hand_read.is_empty(),
        "a workflow cuts a version out of a Cargo manifest:\n{}\n\
         A crate's version is cargo's answer, not a manifest's text. This workspace publishes on \
         more than one version line, so a single read of the root manifest names the wrong \
         version for some crate and names it silently. Let `cargo publish` carry the version it \
         is already holding, or ask `cargo metadata --format-version 1 --no-deps` per package.",
        hand_read.join("\n")
    );
}

const PUBLICATION_ADR: &str = "docs/adr/0015-crates-io-publication-and-semver.md";
const PUBLISHABLE_SET_SECTION: &str = "**Publishable set & order";

// One bold-lead-in section of an ADR, up to the next lead-in or heading. A
// section runs to the next lead-in and not to the next blank line, so it keeps
// its continuation paragraphs — but a lead-in wrapped onto a second physical
// line would be invisible here, and would silently extend the section before
// it.
fn adr_section(text: &str, lead_in: &str) -> Option<String> {
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines() {
        if out.is_empty() {
            if line.starts_with(lead_in) {
                out.push(line);
            }
            continue;
        }
        if line.starts_with("**") || line.starts_with("## ") {
            break;
        }
        out.push(line);
    }
    (!out.is_empty()).then(|| out.join("\n"))
}

/// Is `name` in `text` as a whole crate name? `aozora-flavored-markdown` is a
/// prefix of three others here, so a substring match would let one mention
/// answer for all four.
fn names_crate(text: &str, name: &str) -> bool {
    let bounded = |ch: Option<char>| {
        !ch.is_some_and(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_')
    };
    text.match_indices(name).any(|(at, _)| {
        bounded(text[..at].chars().next_back()) && bounded(text[at + name.len()..].chars().next())
    })
}

#[test]
fn the_publication_adr_accounts_for_every_member_of_this_workspace() {
    // The decision half. ADR-0015 is where this repo says which crates go to
    // crates.io and which stay, and it named two of four for as long as the
    // workflow did — the same drift, written where a reader looks for the
    // reason rather than the command. Vale exempts `docs/adr/*` from the
    // retired-name rule on purpose (a decision record is dated), so no prose
    // gate reaches this file and nothing else here reads it (DEV-237).
    //
    // Over every member and not only the published ones: an ADR that lists
    // what goes and forgets to say why the rest stays is how a crate ends up
    // `publish = false` with nobody able to find out whether that was decided.
    let text = read(PUBLICATION_ADR);
    let section = adr_section(&text, PUBLISHABLE_SET_SECTION).unwrap_or_else(|| {
        panic!(
            "{PUBLICATION_ADR} has no `{PUBLISHABLE_SET_SECTION}` section any more. It is the \
             one place the publishable set is decided rather than executed; if it was renamed, \
             retarget this rule rather than leaving it reading nothing."
        )
    });

    let members = workspace_members();
    let missing: Vec<String> = members
        .iter()
        .map(member_name)
        .filter(|name| !names_crate(&section, name))
        .collect();
    assert!(
        missing.is_empty(),
        "{PUBLICATION_ADR}'s publishable-set section does not account for {missing:?}.\n\
         Every member is either published or `publish = false`, and both are decisions. A member \
         the section never names is one whose status nobody decided — which is how the EPUB pair \
         sat in this workspace, publishable by its manifests, reachable by no release path, with \
         the ADR still describing a two-rung ladder.",
    );
}

// ---------------------------------------------------------------------------
// what the ladder reader claims, pinned both ways
// ---------------------------------------------------------------------------

#[test]
fn a_workspace_publish_is_the_manifests_answer_and_an_exclude_narrows_it() {
    let publishable: BTreeSet<String> = ["alpha", "beta", "gamma"]
        .into_iter()
        .map(str::to_owned)
        .collect();
    let call = |run: &str| {
        let tokens = shell_tokens(run);
        let at = publish_at(&tokens)
            .unwrap_or_else(|| panic!("`{run}` reads as no `cargo publish` at all"));
        PublishCall::read("publish".to_owned(), run.to_owned(), None, &tokens[at..])
    };

    let whole = call("cargo publish --workspace --locked");
    assert_eq!(whole.selection, Selection::Workspace(BTreeSet::new()));
    assert_eq!(crates_reached(&[&whole], &publishable).0, publishable);

    // `--exclude` is the knob a recovery run reaches for — it is cargo's own
    // answer to the resumability the deleted 404 probe used to buy — so it has
    // to subtract rather than read as noise. Otherwise a workflow that skips a
    // crate on purpose still passes here as publishing all of them.
    let narrowed = call("cargo publish --workspace --exclude beta --locked");
    let (reached, opaque) = crates_reached(&[&narrowed], &publishable);
    assert!(opaque.is_empty(), "{opaque:?}");
    assert!(
        !reached.contains("beta") && reached.len() == 2,
        "`--exclude` did not narrow the set cargo would take: {reached:?}"
    );

    // And both spellings of an explicit ladder, which is the fallback ADR-0015
    // records: readable, and compared against the manifests like any other.
    let listed = call("cargo publish -p alpha --package gamma --locked");
    assert_eq!(
        crates_reached(&[&listed], &publishable).0,
        ["alpha", "gamma"]
            .into_iter()
            .map(str::to_owned)
            .collect::<BTreeSet<String>>()
    );
}

#[test]
fn the_ladder_that_stood_before_this_reader_could_not_be_compared_to_the_manifests() {
    // The workflow as it was, verbatim in the shapes that matter: a preflight
    // that dry-ran the leaf alone, and an upload whose crate came from a shell
    // variable fed by a two-name `for`. Both rules above fail on it, and they
    // fail for the two different reasons it was wrong.
    let before = concat!(
        "jobs:\n",
        "  package:\n",
        "    steps:\n",
        "      - name: Dry-run aozora-flavored-markdown (metadata smoke test)\n",
        "        run: cargo publish -p aozora-flavored-markdown --dry-run --locked\n",
        "  publish:\n",
        "    needs: package\n",
        "    steps:\n",
        "      - name: Publish to crates.io (topological, resumable)\n",
        "        run: |\n",
        "          for crate in aozora-flavored-markdown aozora-flavored-markdown-cli; do\n",
        "            if out=\"$(cargo publish -p \"${crate}\" --locked 2>&1)\"; then\n",
        "              echo \"::notice::published ${crate}\"\n",
        "            fi\n",
        "          done\n",
    );
    // No `Justfile` behind these fixtures: the shape under test spelled every
    // command out where it ran it, which is the state that made the recipe
    // hop worth reading in the first place.
    let calls = publish_calls(before, "");
    assert_eq!(calls.len(), 2, "the reader no longer finds both calls");

    let publishable = publishable_crates();
    let packaged: Vec<&PublishCall> = calls.iter().filter(|call| call.dry_run).collect();
    let (verified, dry_opaque) = crates_reached(&packaged, &publishable);
    assert!(dry_opaque.is_empty(), "{dry_opaque:?}");
    assert_eq!(
        verified.len(),
        1,
        "the preflight packaged one crate and the reader saw {verified:?}"
    );

    let uploading: Vec<&PublishCall> = calls.iter().filter(|call| !call.dry_run).collect();
    let (_, live_opaque) = crates_reached(&uploading, &publishable);
    assert_eq!(
        live_opaque.len(),
        1,
        "a ladder that names its crates in a shell variable read as a decidable set: \
         {live_opaque:?}"
    );

    // And the same ladder with its two names written out, which is the state
    // the rule has to bite on for reasons of substance rather than spelling:
    // readable, comparable, and two crates short.
    let literal = publish_calls(
        concat!(
            "jobs:\n",
            "  publish:\n",
            "    steps:\n",
            "      - run: cargo publish -p aozora-flavored-markdown --locked\n",
            "      - run: cargo publish -p aozora-flavored-markdown-cli --locked\n",
        ),
        "",
    );
    let listed: Vec<&PublishCall> = literal.iter().collect();
    let (uploaded, opaque) = crates_reached(&listed, &publishable);
    assert!(opaque.is_empty(), "{opaque:?}");
    assert_ne!(
        uploaded, publishable,
        "the two-rung ladder read as covering every publishable crate; it covers {uploaded:?} \
         of {publishable:?}"
    );
    assert!(
        verified.len() < uploaded.len(),
        "the preflight covered {verified:?} and the ladder uploaded {uploaded:?}; the gap this \
         rule was written for is not being measured"
    );
}

/// A Justfile in the shape this repo's packaging now has: the dry run inside a
/// gate, reached through a dependency, behind an unexpanded `{{VAR}}`.
const PACKAGING_GATE_WITH_A_DEPENDENCY: &str = concat!(
    "_DEV := \"docker compose run --rm dev\"\n",
    "\n",
    "[group('gate')]\n",
    "package: build\n",
    "    {{_DEV}} cargo publish --workspace --dry-run --locked --allow-dirty\n",
    "\n",
    "build:\n",
    "    {{_DEV}} cargo build --locked --workspace\n",
);

#[test]
fn a_step_that_names_a_recipe_is_read_as_the_command_that_recipe_runs() {
    // Two readers hop from a `just <recipe>` into the recipe body, and both
    // exist because the dry run stopped being written where it runs. A reader
    // that stopped at the `just` would find no preflight in the publish
    // workflow at all — and "no preflight" is how the rule above spells the
    // failure it was written to catch, so the arrangement that put packaging
    // in front of every merge would be reported as the one that packages
    // nothing.
    let justfile = PACKAGING_GATE_WITH_A_DEPENDENCY;

    // A dependency's body counts as the recipe's, and `{{VAR}}` resolves —
    // otherwise `{{_DEV}} cargo publish` reads as a command starting at a
    // token no tokenizer can see past.
    let commands = recipe_commands(justfile, "package");
    assert!(
        commands
            .iter()
            .any(|line| line.contains("docker compose run --rm dev cargo publish")),
        "the recipe's own line went unresolved: {commands:?}"
    );
    assert!(
        commands.iter().any(|line| line.contains("cargo build")),
        "a dependency's body was not read as part of the recipe: {commands:?}"
    );
    assert!(
        recipe_commands(justfile, "no-such-recipe").is_empty(),
        "a recipe that does not exist answered with somebody else's lines"
    );

    let workflow = concat!(
        "jobs:\n",
        "  package:\n",
        "    steps:\n",
        "      - run: just package\n",
        "  publish:\n",
        "    needs: package\n",
        "    steps:\n",
        "      - run: cargo publish --workspace --locked\n",
    );
    let calls = publish_calls(workflow, justfile);
    assert_eq!(
        calls.len(),
        2,
        "the reader found {} `cargo publish`(es) across a step that names a recipe and a step \
         that writes the command out",
        calls.len()
    );

    let through: Vec<&PublishCall> = calls.iter().filter(|call| call.dry_run).collect();
    assert_eq!(
        through.len(),
        1,
        "the dry run behind the recipe went unread"
    );
    assert_eq!(
        through[0].via.as_deref(),
        Some("package"),
        "the call was found but not attributed to the recipe that holds it, so a rule asking \
         whether a gate owns it has nothing to ask about"
    );
    assert!(
        through[0].command.contains("just package") && through[0].command.contains("cargo publish"),
        "the failure message names one of the two lines and not both: {}",
        through[0].command
    );
    assert!(
        through[0].has("--allow-dirty"),
        "a flag inside the recipe did not reach the call the workflow drives"
    );

    // The upload is written where it runs, so it carries no recipe — and a
    // rule that demanded one of every call would demand a recipe that pushes
    // to crates.io.
    let live: Vec<&PublishCall> = calls.iter().filter(|call| !call.dry_run).collect();
    assert_eq!(live.len(), 1);
    assert!(
        live[0].via.is_none(),
        "a command a workflow spells out was attributed to a recipe"
    );
}

#[test]
fn a_publish_reads_as_a_gates_only_where_the_group_attribute_says_it_is_one() {
    // The gate-side reader answers off the Justfile alone, which is what makes
    // "does a pull request package this" a question about the manifest of
    // gates rather than about a workflow. What it must not do is answer yes to
    // a recipe that merely exists: `package` also carries `[group('release')]`,
    // and a reader taking any group would call an ungated packaging recipe a
    // gate — the exact claim the rule above is making.
    let justfile = PACKAGING_GATE_WITH_A_DEPENDENCY;
    let gated = publish_calls_from_gates(justfile);
    assert_eq!(
        gated.len(),
        1,
        "the gate's own publish went unread: {}",
        gated.len()
    );
    assert!(gated[0].dry_run && gated[0].via.as_deref() == Some("package"));
    assert!(
        gated[0].has("--allow-dirty"),
        "the recipe's flags did not reach the call read off it"
    );
    assert!(
        publish_calls_from_gates(&justfile.replace("[group('gate')]\n", "")).is_empty(),
        "a recipe in no group read as a gate, so an ungated packaging recipe would satisfy the \
         rule that asks for a gated one"
    );
    assert!(
        publish_calls_from_gates(&justfile.replace("[group('gate')]", "[group('release')]"))
            .is_empty(),
        "a recipe in a group that is not `gate` read as one"
    );
}

#[test]
fn a_bare_publish_names_no_set_and_a_version_grep_is_told_from_a_paths_filter() {
    // A `cargo publish` with no selector is not "the whole workspace by
    // default" — it is whichever crate the working directory holds, which in a
    // virtual manifest root is an error and one directory down is a silent
    // ladder of one.
    let bare = publish_selection(&shell_tokens("cargo publish --locked"));
    assert!(
        matches!(bare, Selection::Opaque(_)),
        "a publish naming no package read as a set: {bare:?}"
    );

    // The line as the workflow carried it.
    assert!(reads_a_manifest_version(
        "          ver=\"$(grep -m1 '^version' Cargo.toml | sed -E 's/.*\"([^\"]+)\".*/\\1/')\""
    ));
    // And the three shapes a workflow legitimately writes a version or a
    // manifest in: a paths filter, prose, and release-pins.yml's read of the
    // cargo-dist pin out of `dist-workspace.toml` — a tool's own version, in a
    // file that is not a Cargo manifest. Reading any of them as a crate
    // version would make this rule unlivable, and an unlivable rule gets
    // switched off rather than obeyed.
    assert!(!reads_a_manifest_version("              - 'Cargo.toml'"));
    assert!(!reads_a_manifest_version(
        "# a file no updater reads (Dependabot's cargo ecosystem parses Cargo.toml / version)"
    ));
    assert!(!reads_a_manifest_version(
        "          have=\"v$(grep -oE 'cargo-dist-version = \"[0-9.]+\"' dist-workspace.toml)\""
    ));
}

#[test]
fn an_adr_section_ends_where_the_next_one_starts_and_a_crate_name_is_read_whole() {
    let adr = concat!(
        "**Publishable set & order (amended).** `alpha` and `alpha-cli` go up.\n",
        "`alpha-wasm` stays `publish = false`.\n",
        "\n",
        "**Automation (amended).** One command: `beta`.\n",
    );
    let section = adr_section(adr, PUBLISHABLE_SET_SECTION).expect("the lead-in went unread");
    assert!(
        names_crate(&section, "alpha-wasm"),
        "a name in the section's second line went unread"
    );
    assert!(
        !names_crate(&section, "beta"),
        "the next section's content leaked into this one, so a name recorded anywhere in the \
         ADR would answer for the publishable set"
    );
    // The prefix trap: three of this workspace's four published crates start
    // with the name of the fourth, so a `contains` would let one backtick pair
    // account for all of them.
    assert!(
        !names_crate("only `alpha-cli` is here", "alpha"),
        "a longer crate name answered for the shorter one it starts with"
    );
    assert!(names_crate("only `alpha-cli` is here", "alpha-cli"));
    assert!(
        adr_section(adr, "**Nothing writes this").is_none(),
        "a section that is not there read as present"
    );
}

// ---------------------------------------------------------------------------
// what leaves inside the tarball
// ---------------------------------------------------------------------------
//
// The section above is the fourth reading of `publish = false` and asks WHICH
// crates leave this workspace. Every rule it holds is about a set of names, and
// the set was right while all four tarballs were wrong: each carried the
// repository's landing README (an inherited `readme` resolves against the
// WORKSPACE root, so one document was four crates.io pages, and the EPUB pair's
// page never said EPUB), none carried a word of either licence though `license`
// promises two, one shipped a `#[cfg(doctest)]` `include_str!` reaching three
// directories above its own package, and most of every tarball's bytes were a
// test suite only this repository runs (DEV-225).
//
// None of that is a gap in those rules — it is the same gap `just package` has.
// That gate unpacks each crate and BUILDS it, which is the strongest thing any
// gate here does to a tarball, and every defect above survives it: a wrong
// README compiles, an absent licence compiles, an excluded test suite compiles,
// and a `#[cfg(doctest)]` or `#[cfg(test)]` include is not compiled by a verify
// build at all — those configurations are exactly the ones `cargo build` turns
// off, so the one include that escaped the package was the one the packaging
// gate could not see.
//
// So these rules ask cargo what it would put in the archive — `cargo package
// --list` is the same collection step the gate runs, minus the build — and hold
// the answer to what a consumer needs out of it. `--allow-dirty` for the reason
// the recipe gives: committedness is a different question, asked on the upload.

/// The document a manifest puts on a crate's page, as it spells the key.
#[derive(Debug, PartialEq, Eq)]
enum Readme {
    /// `readme = "…"`, resolved against the crate's own directory.
    Own(String),
    /// `readme.workspace = true` or `readme = { workspace = true }`. Cargo
    /// resolves the inherited value against the WORKSPACE root — which is the
    /// whole defect, because the value that arrives is a path and the crate it
    /// arrives at is not where it was written.
    Inherited,
    /// No key at all. Cargo still ships a `README.md` sitting beside the
    /// manifest, so absence is not the same as shipping nothing.
    Absent,
}

fn readme_declaration(manifest: &str) -> Readme {
    if manifest_value(manifest, "package", "readme.workspace").is_some() {
        return Readme::Inherited;
    }
    match manifest_value(manifest, "package", "readme") {
        Some(value) if value.contains("workspace") => Readme::Inherited,
        Some(value) => quoted_items(value)
            .first()
            .map_or(Readme::Absent, |path| Readme::Own(path.clone())),
        None => Readme::Absent,
    }
}

/// The README one member ships, repo-relative, by cargo's own resolution.
///
/// `workspace_readme` is passed rather than read so the reader can be shown the
/// arrangement this repo used to have — that is the case the rules below are
/// about, and it is no longer reachable from the manifests on disk.
fn shipped_readme(
    member_path: &str,
    manifest: &str,
    workspace_readme: Option<&str>,
) -> Option<String> {
    match readme_declaration(manifest) {
        Readme::Own(path) => Some(format!("{member_path}/{path}")),
        Readme::Inherited => workspace_readme.map(str::to_owned),
        Readme::Absent => {
            let beside = format!("{member_path}/README.md");
            repo_root().join(&beside).is_file().then_some(beside)
        }
    }
}

/// The `readme` the workspace table offers members, if it offers one.
fn workspace_readme() -> Option<String> {
    let root = read("Cargo.toml");
    manifest_value(&root, "workspace.package", "readme")
        .and_then(|value| quoted_items(value).first().cloned())
}

/// Every link target a Markdown document points at, with its 1-based line.
/// Fenced blocks are skipped: a path inside one is a sample, not a link.
fn markdown_link_targets(text: &str) -> Vec<(usize, String)> {
    let inline = Regex::new(r"\]\(\s*<?([^)>\s]+)").expect("the inline-link reader is a regex");
    let reference =
        Regex::new(r"^ {0,3}\[[^\]]+\]:\s*<?([^\s>]+)").expect("the reference reader is a regex");
    let mut fenced = false;
    let mut out = Vec::new();
    for (at, line) in text.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed.starts_with("```") || trimmed.starts_with("~~~") {
            fenced = !fenced;
            continue;
        }
        if fenced {
            continue;
        }
        let hits = inline
            .captures_iter(line)
            .chain(reference.captures_iter(line));
        out.extend(hits.map(|capture| (at + 1, capture[1].to_owned())));
    }
    out
}

/// Can a reader on crates.io follow this target? That page is not on GitHub and
/// not in a checkout, so a repo-relative path is a 404 there and nowhere else.
fn followable_from_crates_io(target: &str) -> bool {
    target.starts_with("https://")
        || target.starts_with("http://")
        || target.starts_with('#')
        || target.starts_with("mailto:")
}

/// The files `cargo package` would collect for one crate, as paths inside the
/// archive. Asked of cargo rather than derived: `exclude`, the auto-discovered
/// targets and the symlinks the licence files are all resolve here, and a
/// second implementation of that is what would drift.
fn packaged_files(crate_name: &str) -> BTreeSet<String> {
    let out = Command::new("cargo")
        .args(["package", "--list", "--locked", "--allow-dirty", "--quiet"])
        .args(["-p", crate_name])
        .current_dir(repo_root())
        .output()
        .unwrap_or_else(|e| panic!("running `cargo package --list -p {crate_name}`: {e}"));
    assert!(
        out.status.success(),
        "`cargo package --list -p {crate_name}` failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let files: BTreeSet<String> = String::from_utf8_lossy(&out.stdout)
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(str::to_owned)
        .collect();
    assert!(
        files.contains("Cargo.toml"),
        "`cargo package --list -p {crate_name}` came back without even a manifest ({files:?}); a \
         reader finding nothing calls every tarball complete"
    );
    files
}

/// The identifiers an SPDX expression names.
fn spdx_ids(expression: &str) -> Vec<String> {
    expression
        .split(|ch: char| ch.is_whitespace() || ch == '(' || ch == ')')
        .filter(|word| !word.is_empty() && !matches!(*word, "OR" | "AND" | "WITH"))
        .map(str::to_owned)
        .collect()
}

/// The file this repo keeps one identifier's text in. `None` is a failure and
/// not a pass: an identifier nothing here can point at is a promise on the
/// crates.io page with no text behind it.
fn licence_file(id: &str) -> Option<&'static str> {
    match id {
        "Apache-2.0" => Some("LICENSE-APACHE"),
        "MIT" => Some("LICENSE-MIT"),
        _ => None,
    }
}

/// The SPDX expression a member publishes under, inherited or its own.
fn licence_expression(manifest: &str, workspace: &str) -> String {
    manifest_value(manifest, "package", "license")
        .filter(|value| !value.contains("workspace"))
        .and_then(|value| quoted_items(value).first().cloned())
        .unwrap_or_else(|| workspace.to_owned())
}

/// Every `include_str!` / `include_bytes!` path in one source file, with its
/// 1-based line. A comment-only line is skipped: prose about an include is not
/// one, and this file's subject is prose that reads like enforcement.
fn included_paths(src: &str) -> Vec<(usize, String)> {
    let include = Regex::new(r#"include_(?:str|bytes)!\s*\(\s*"([^"]+)""#)
        .expect("the include reader is a regex");
    let mut out = Vec::new();
    for (at, line) in src.lines().enumerate() {
        if line.trim_start().starts_with("//") {
            continue;
        }
        out.extend(
            include
                .captures_iter(line)
                .map(|capture| (at + 1, capture[1].to_owned())),
        );
    }
    out
}

/// `relative` applied to `base` by folding `.` and `..` textually. Lexical
/// because the path being resolved is the one suspected of not existing.
fn lexical_join(base: &Path, relative: &str) -> PathBuf {
    let mut out = base.to_path_buf();
    for part in relative.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                out.pop();
            }
            _ => out.push(part),
        }
    }
    out
}

/// A directory cargo auto-discovers targets in that only this repository ever
/// runs, plus the fuzz crate's own workspace. `cargo test` and `cargo bench`
/// on a DEPENDENCY run none of these, so every byte of them on crates.io is
/// bandwidth spent on nobody.
const TARGETS_ONLY_THIS_REPO_RUNS: &[&str] = &["tests", "benches", "fuzz"];

/// Why an `include_str!` in published source may still reach outside its own
/// package. One entry is one tarball a consumer cannot `cargo test`.
const CONFORMANCE_FIXTURES: &str = "the CommonMark / GFM fixtures live at the repository root and are shared with `xtask \
     spec-refresh`, so no copy of them sits inside this package. The module is `#[cfg(test)]`, \
     which is why nothing has ever reported it: `cargo package`'s verify build compiles with \
     `cargo build`, where that configuration is off. It is the same blind spot DEV-225 closed \
     for the `#[cfg(doctest)]` README include, left open one file over — a SURVIVING DEFECT, \
     not a decision.";

/// The escapes that stand, as `(source file, include path, why)`.
const INCLUDES_THAT_LEAVE_THE_PACKAGE: &[(&str, &str, &str)] = &[
    (
        "crates/aozora-flavored-markdown/src/conformance.rs",
        "../../../spec/commonmark-0.31.2.json",
        CONFORMANCE_FIXTURES,
    ),
    (
        "crates/aozora-flavored-markdown/src/conformance.rs",
        "../../../spec/gfm-0.29-gfm.json",
        CONFORMANCE_FIXTURES,
    ),
];

#[test]
fn every_crate_this_repo_publishes_ships_a_readme_of_its_own() {
    // The root cause first. `[workspace.package] readme` is a path resolved
    // against the workspace root by every member that inherits it, so the key
    // cannot mean "each crate's README" no matter how it is spelled. Leaving it
    // undefined is what turns `readme.workspace = true` from a plausible-looking
    // line into a hard cargo error.
    assert!(
        workspace_readme().is_none(),
        "[workspace.package] offers a `readme` for members to inherit. An inherited one resolves \
         against the WORKSPACE root, so every member taking it publishes the same document — \
         which is how four crates.io pages came to be one repository landing page."
    );

    let members = workspace_members();
    let inheritable = workspace_readme();
    let mut shipped: BTreeMap<String, String> = BTreeMap::new();
    let mut opted_out = 0_usize;
    for member in &members {
        assert_ne!(
            readme_declaration(&member.manifest),
            Readme::Inherited,
            "{}/Cargo.toml inherits its `readme`, which resolves against the workspace root.",
            member.path
        );
        if !is_published(&member.manifest) {
            opted_out += 1;
            continue;
        }
        let resolved = shipped_readme(&member.path, &member.manifest, inheritable.as_deref());
        let path = resolved.unwrap_or_else(|| {
            panic!(
                "{}/Cargo.toml publishes to crates.io and names no README. Its page would be the \
                 crate name, the description and nothing else.",
                member.path
            )
        });
        assert!(
            repo_root().join(&path).is_file(),
            "{}/Cargo.toml names `{path}`, which is not a file. Cargo fails the package, but only \
             once somebody packages it.",
            member.path
        );
        let name = member_name(member);
        assert!(
            read(&path).contains(&name),
            "`{path}` is what `{name}`'s crates.io page shows and it never names that crate. That \
             is the EPUB page verbatim: a document about something else, in front of the reader \
             who came for this."
        );
        if let Some(other) = shipped.insert(path.clone(), name.clone()) {
            panic!(
                "`{path}` is the crates.io page of both `{other}` and `{name}`. One document \
                 cannot be the introduction to two crates — the one it is not about is the one \
                 nobody notices."
            );
        }
    }

    // Blindness check, both ways, as the docs.rs rule makes it: a reader that
    // cannot tell `publish = false` from its absence answers every assertion
    // above over the whole workspace or over nothing.
    assert!(
        shipped.len() >= 2 && opted_out >= 1,
        "{} published and {opted_out} opted-out member(s) out of {}; the reader is not telling \
         `publish = false` apart from its absence",
        shipped.len(),
        members.len()
    );
}

#[test]
fn every_link_a_published_readme_carries_is_one_a_crates_io_reader_can_follow() {
    // A README is the one document this repo publishes to a place that is not
    // this repo. `./docs/adr/`, `./LICENSE-MIT`, `./CONTRIBUTING.md` all resolve
    // on GitHub and 404 from crates.io, and nothing here could say so: `just
    // vale` reads these files for retired names and never resolves a target,
    // and the rustdoc gates' `broken_intra_doc_links` is about Rust paths in
    // Rust docs — the README was not in rustdoc at all.
    let members = workspace_members();
    let inheritable = workspace_readme();
    let mut read_targets = 0_usize;
    let mut unreachable: Vec<String> = Vec::new();
    for member in &members {
        // Published, or shipping a README to somebody anyway: the wasm crate's
        // reaches editor hosts through `pkg/`, which `wasm-pack` fills from
        // these same fields.
        let declares_own = matches!(readme_declaration(&member.manifest), Readme::Own(_));
        if !is_published(&member.manifest) && !declares_own {
            continue;
        }
        let resolved = shipped_readme(&member.path, &member.manifest, inheritable.as_deref());
        let Some(path) = resolved else {
            continue;
        };
        for (line, target) in markdown_link_targets(&read(&path)) {
            read_targets += 1;
            if !followable_from_crates_io(&target) {
                unreachable.push(format!("{path}:{line}: {target}"));
            }
        }
    }
    assert!(
        read_targets >= 20,
        "{read_targets} link(s) read across every README this repo publishes; the reader is not \
         finding them, and a reader finding nothing calls every page followable"
    );
    assert!(
        unreachable.is_empty(),
        "these link(s) resolve in a checkout and 404 from the page they are published on:\n  \
         {}\nA published README is read where the repository is not. Spell the target as an \
         absolute URL.",
        unreachable.join("\n  ")
    );
}

#[test]
fn every_crate_this_repo_publishes_carries_the_licence_text_it_names() {
    // `license = "Apache-2.0 OR MIT"` is an identifier, and cargo carries no
    // text with one. Until DEV-225 the text existed at the repository root and
    // nowhere else, so every tarball this workspace would have uploaded offered
    // a consumer two licences and the words of neither.
    //
    // The two gates that read the word "licence" both point the other way:
    // `just deny` answers what this repo may take IN, and dependency-review.yml
    // answers the same about a pull request. Nothing asked about the licence
    // going OUT, which is the only one this repo is the author of.
    let root = read("Cargo.toml");
    let expression = manifest_value(&root, "workspace.package", "license")
        .and_then(|value| quoted_items(value).first().cloned())
        .expect("[workspace.package] declares no `license` for the members to inherit");
    let mut checked = 0_usize;
    for member in workspace_members() {
        if !is_published(&member.manifest) {
            continue;
        }
        let ids = spdx_ids(&licence_expression(&member.manifest, &expression));
        assert!(
            !ids.is_empty(),
            "{}/Cargo.toml resolves to an empty licence expression",
            member.path
        );
        let packaged = packaged_files(&member_name(&member));
        // `NOTICE` rides along with the identifiers: Apache-2.0 §4(d) makes it
        // part of what a redistributor has to receive, and it is the file that
        // names the upstream work this crate is built on.
        let mut wanted: Vec<String> = ids
            .iter()
            .map(|id| {
                licence_file(id)
                    .unwrap_or_else(|| {
                        panic!(
                            "{} publishes under `{id}`, which this reader cannot point at a file \
                             for. Add the text and teach `licence_file` where it is — an \
                             identifier with no text behind it is a promise on a page.",
                            member.path
                        )
                    })
                    .to_owned()
            })
            .collect();
        wanted.push("NOTICE".to_owned());
        for file in wanted {
            assert!(
                packaged.contains(&file),
                "`{}`'s tarball carries no `{file}`. It is one `cargo package --list` away from \
                 being known, and it is the file a consumer's own compliance review asks for \
                 first.",
                member.path
            );
            let beside = read(&format!("{}/{file}", member.path));
            assert_eq!(
                beside,
                read(&file),
                "{}/{file} is not the text at the repository root. Two copies of a licence are \
                 one copy and one claim about it.",
                member.path
            );
            checked += 1;
        }
    }
    assert!(
        checked >= 8,
        "{checked} licence file(s) checked across every published crate; the reader is not \
         finding the published set"
    );
}

#[test]
fn no_tarball_this_repo_publishes_carries_a_target_only_this_repo_runs() {
    // `cargo package` collects everything git tracks under the package unless
    // `exclude` says otherwise, so the default is that a consumer downloads the
    // test suite, the benchmarks and the fuzz corpus and can run none of them —
    // `cargo test` on a DEPENDENCY builds no test target of that dependency.
    // Nothing measured it: the packaging gate reports a size and has no ceiling,
    // and a tarball is not less buildable for being four times too big.
    let mut with_such_a_directory = 0_usize;
    let mut carried: Vec<String> = Vec::new();
    for member in workspace_members() {
        if !is_published(&member.manifest) {
            continue;
        }
        if TARGETS_ONLY_THIS_REPO_RUNS
            .iter()
            .any(|dir| repo_root().join(&member.path).join(dir).is_dir())
        {
            with_such_a_directory += 1;
        }
        // An `exclude` entry naming nothing is a rule over nothing — the shape
        // a typo takes, and the shape the whole payload comes back in. Only for
        // the literal paths: cargo takes gitignore patterns here too, and a
        // rule that cannot read one would be a rule that has to be switched off
        // the first time somebody writes one.
        for entry in manifest_value(&member.manifest, "package", "exclude")
            .map(quoted_items)
            .unwrap_or_default()
        {
            if entry.contains(['*', '?', '[', '!']) {
                continue;
            }
            let path = repo_root()
                .join(&member.path)
                .join(entry.trim_end_matches('/'));
            assert!(
                path.exists(),
                "{}/Cargo.toml excludes `{entry}`, which does not exist. An exclusion that \
                 matches nothing looks exactly like one that works.",
                member.path
            );
        }
        let packaged = packaged_files(&member_name(&member));
        carried.extend(packaged.into_iter().filter(|file| {
            TARGETS_ONLY_THIS_REPO_RUNS
                .iter()
                .any(|dir| file.starts_with(&format!("{dir}/")))
        }));
    }
    assert!(
        with_such_a_directory >= 2,
        "only {with_such_a_directory} published crate(s) have a `tests/`, `benches/` or `fuzz/` \
         directory at all; this rule is measuring nothing"
    );
    assert!(
        carried.is_empty(),
        "these files ship to every consumer and are runnable by nobody but this repository:\n  \
         {}\nName the directory in that crate's `exclude`.",
        carried.join("\n  ")
    );
}

#[test]
fn every_file_a_published_source_includes_is_one_its_own_tarball_carries() {
    // An `include_str!` is a build dependency on a file, written in a form no
    // manifest records. Inside the package it is free; outside it, the source
    // ships and the file it reads does not, and the crate a consumer unpacked
    // cannot be built in the configuration that reaches the line.
    //
    // The packaging gate is the one that should hold this and structurally
    // cannot: it verifies by BUILDING, and both spellings this workspace used
    // — `#[cfg(doctest)]` on the README include, `#[cfg(test)]` on the spec
    // runners — are configurations a verify build turns off. `just test-doc`
    // compiles the first, from a working tree where three directories up is
    // still the repository.
    let mut standing: BTreeSet<(String, String)> = INCLUDES_THAT_LEAVE_THE_PACKAGE
        .iter()
        .map(|&(file, path, _)| (file.to_owned(), path.to_owned()))
        .collect();
    let mut escapes: Vec<String> = Vec::new();
    let mut resolved = 0_usize;
    for member in workspace_members() {
        if !is_published(&member.manifest) {
            continue;
        }
        let crate_dir = repo_root().join(&member.path);
        let packaged = packaged_files(&member_name(&member));
        for source in rust_files(&crate_dir.join("src")) {
            let relative = format!(
                "{}/{}",
                member.path,
                source
                    .strip_prefix(&crate_dir)
                    .unwrap_or(&source)
                    .to_string_lossy()
                    .replace('\\', "/")
            );
            let directory = source.parent().unwrap_or(&crate_dir).to_path_buf();
            for (line, raw) in included_paths(&fs::read_to_string(&source).unwrap_or_default()) {
                resolved += 1;
                if standing.remove(&(relative.clone(), raw.clone())) {
                    continue;
                }
                let target = lexical_join(&directory, &raw);
                let inside = target
                    .strip_prefix(&crate_dir)
                    .ok()
                    .map(|path| path.to_string_lossy().replace('\\', "/"));
                match inside {
                    Some(path) if packaged.contains(&path) => {}
                    Some(path) => escapes.push(format!(
                        "{relative}:{line}: `{raw}` is `{path}`, which `exclude` \
                                       keeps out of the tarball"
                    )),
                    None => escapes.push(format!(
                        "{relative}:{line}: `{raw}` reaches outside the package"
                    )),
                }
            }
        }
    }
    assert!(
        resolved >= 3,
        "{resolved} include(s) read across every published crate; the reader is not finding them"
    );
    assert!(
        escapes.is_empty(),
        "these included files are not in the tarball that ships the source reading them:\n  {}",
        escapes.join("\n  ")
    );
    // An exemption for an escape that is gone is an exemption that would excuse
    // the next one silently.
    assert!(
        standing.is_empty(),
        "INCLUDES_THAT_LEAVE_THE_PACKAGE still excuses {standing:?}, which no published source \
         writes any more. Delete the entry."
    );
}

/// Every figure the conformance runners measure and this repository's prose
/// restates, as (what the figure is, a pattern whose one capture is the number
/// stating it).
///
/// Patterns rather than a table of document-and-value: the figures are allowed
/// to change — a spec refresh moves them — and no manifest declares where a
/// sentence lives. What is not allowed is two documents answering the same
/// question differently.
const CONFORMANCE_FIGURES: &[(&str, &str)] = &[
    (
        "the CommonMark suite total",
        r"all (\d+) CommonMark 0\.31\.2",
    ),
    (
        "the CommonMark suite total",
        r"CommonMark 0\.31\.2 \((\d+) cases",
    ),
    ("the CommonMark suite total", r"pass = (\d+)/\d+"),
    ("the CommonMark suite total", r"pass = \d+/(\d+)"),
    ("the GFM suite total", r"all (\d+) GFM 0\.29"),
    ("the GFM suite total", r"GFM 0\.29 \((\d+),"),
    ("the GFM suite total", r"of the (\d+) come out verbatim"),
    (
        "the GFM examples that come back verbatim",
        r"(\d+) of the \d+ come out verbatim",
    ),
    (
        "the GFM examples that come back verbatim",
        r"(\d+) verbatim,",
    ),
    (
        "the GFM examples pinned to a later authority",
        r"the last (\d+) are pinned",
    ),
    (
        "the GFM examples pinned to a later authority",
        r"(\d+) pinned to what supersedes",
    ),
    (
        "the GFM examples pinned to a later authority",
        r"of which (\d+) are pinned",
    ),
    (
        "the GFM examples pinned to a later authority",
        r"the list of (\d+)",
    ),
];

/// The one document whose figures are held against the measurement itself, by
/// `conformance::the_crate_page_states_the_figures_this_file_measures` — which
/// can reach it because it is the page inside the package the suite lives in.
/// Every other copy is checked against this one, so agreement is agreement
/// with something that was measured rather than between two guesses.
const PAGE_PINNED_TO_THE_MEASUREMENT: &str = "crates/aozora-flavored-markdown/README.md";

/// One document on one line, with a leading `#` dropped from each. Where a
/// paragraph wraps is a typesetting decision, and the `Justfile` states its
/// figures in a comment block whose line breaks mean no more than a README's.
fn unwrapped_prose(text: &str) -> String {
    text.lines()
        .flat_map(|line| line.trim_start().trim_start_matches('#').split_whitespace())
        .collect::<Vec<_>>()
        .join(" ")
}

/// Every document a reader could meet a conformance figure in: the prose at
/// the repository root, each member's own page, the `docs/` index, and the
/// `Justfile`, whose recipe comments are what `just --list` prints.
///
/// Walked rather than listed, because the defect this rule exists for is a
/// copy nobody knew about. The library crate's page was created by copying the
/// landing README's compatibility claim into it, and a list written that day
/// would have named one document.
///
/// `CHANGELOG.md` and `docs/adr/` are out for the reason the retired-path gate
/// leaves them out: history is allowed to say what was true then.
fn documents_that_could_state_a_figure() -> Vec<String> {
    let mut directories = vec![String::new(), "docs".to_owned()];
    directories.extend(
        workspace_members()
            .iter()
            .map(|member| crate_dir(&member.path).to_owned()),
    );
    let mut out = vec!["Justfile".to_owned()];
    for directory in directories {
        let path = repo_root().join(&directory);
        let entries =
            fs::read_dir(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        for entry in entries {
            let file = entry
                .unwrap_or_else(|e| panic!("reading an entry of {}: {e}", path.display()))
                .path();
            if file.extension().is_some_and(|extension| extension == "md") {
                let Some(name) = file
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                else {
                    continue;
                };
                if name == "CHANGELOG.md" {
                    continue;
                }
                out.push(if directory.is_empty() {
                    name
                } else {
                    format!("{directory}/{name}")
                });
            }
        }
    }
    out.sort();
    out
}

#[test]
fn every_document_that_states_a_conformance_figure_states_the_same_one() {
    // The compatibility claim is this repository's headline and it is written
    // down three times: the landing README, the library crate's page, and the
    // comment on the recipe that proves it. One of the three was wrong for
    // months — it said the GFM suite passed verbatim while the runner asserted
    // 22 of 672 — and every gate stayed green, because prose is not compiled
    // and no two of those files are read by the same thing.
    //
    // `conformance.rs` now holds ONE of them against the measurement. That is
    // the half a `#[cfg(test)]` module inside a published package can do:
    // reaching the other two would mean including files the tarball does not
    // carry, which is its own defect one directory up. This is the other half
    // — every copy says what the pinned copy says — and together they are the
    // claim, its proof, and the fact that they are about each other.
    let documents = documents_that_could_state_a_figure();
    let mut stated: BTreeMap<&str, BTreeSet<(String, String)>> = BTreeMap::new();
    for document in &documents {
        let prose = unwrapped_prose(&read(document));
        for &(figure, pattern) in CONFORMANCE_FIGURES {
            let reader = Regex::new(pattern).unwrap_or_else(|e| panic!("`{pattern}`: {e}"));
            for capture in reader.captures_iter(&prose) {
                stated
                    .entry(figure)
                    .or_default()
                    .insert((capture[1].to_owned(), document.clone()));
            }
        }
    }

    assert!(
        documents.len() >= 8,
        "only {documents:?} read; the walk is not finding this repository's prose"
    );
    let asked: BTreeSet<&str> = CONFORMANCE_FIGURES
        .iter()
        .map(|&(figure, _)| figure)
        .collect();
    assert_eq!(
        stated.keys().copied().collect::<BTreeSet<&str>>(),
        asked,
        "a figure below is stated nowhere any more. Either the prose that carried it was cut — in \
         which case cut its patterns too — or it was reworded past every reader here, which is \
         the state this rule exists to make impossible."
    );

    for (figure, sightings) in &stated {
        let values: BTreeSet<&str> = sightings.iter().map(|(value, _)| value.as_str()).collect();
        assert_eq!(
            values.len(),
            1,
            "{figure} is stated more than one way: {sightings:?}. One of these documents is out of \
             date, and a reader has no way to tell which."
        );
        let pages: BTreeSet<&str> = sightings
            .iter()
            .map(|(_, document)| document.as_str())
            .collect();
        assert!(
            pages.len() >= 2,
            "{figure} is stated in {pages:?} alone, so this rule compares nothing for it"
        );
        assert!(
            pages.contains(PAGE_PINNED_TO_THE_MEASUREMENT),
            "{figure} is stated in {pages:?}, and none of those is \
             {PAGE_PINNED_TO_THE_MEASUREMENT} — the one page held against what the suite actually \
             measures. Agreeing with each other is not the same as being right."
        );
    }
}

// --- the readers above, on the shapes that would fool them ------------------

#[test]
fn the_readme_arrangement_that_stood_before_these_readers_reads_as_the_defect() {
    // The manifests as this repo shipped them, in both spellings it used: the
    // dotted one the library and CLI wrote, and the inline table the EPUB pair
    // wrote. Reconstructed rather than read, because the point of the rules
    // above is that this arrangement can no longer be reached from disk — and a
    // rule whose subject has been deleted is a rule nobody can check.
    let dotted = "[package]\nname = \"lib\"\nreadme.workspace       = true\n";
    let inline = "[package]\nname = \"epub\"\nreadme        = { workspace = true }\n";
    let own = "[package]\nname = \"lib\"\nreadme = \"README.md\"\n";
    assert_eq!(readme_declaration(dotted), Readme::Inherited);
    assert_eq!(readme_declaration(inline), Readme::Inherited);
    assert_eq!(readme_declaration(own), Readme::Own("README.md".to_owned()));
    assert_eq!(
        readme_declaration("[package]\nname = \"x\"\n"),
        Readme::Absent
    );

    // And what the inheritance resolved to: one document, for both of them,
    // and it is not in either crate's directory.
    let workspace = Some("README.md");
    let library = shipped_readme("crates/lib", dotted, workspace);
    let epub = shipped_readme("crates/epub", inline, workspace);
    assert_eq!(library.as_deref(), Some("README.md"));
    assert_eq!(epub, library, "the two crates shipped different documents");
    assert_eq!(
        shipped_readme("crates/lib", own, workspace).as_deref(),
        Some("crates/lib/README.md"),
        "an own `readme` resolved against the workspace root, which is the bug spelled the other \
         way round"
    );

    // The document they landed on is this repository's, and it is written for a
    // reader who has the repository.
    let repo_relative = markdown_link_targets(&read("README.md"))
        .into_iter()
        .any(|(_, target)| !followable_from_crates_io(&target));
    assert!(
        repo_relative,
        "the landing README has no repo-relative link left, so the link rule above can no longer \
         be shown failing on the arrangement it was written for"
    );
}

#[test]
fn a_link_inside_a_code_fence_is_not_one_and_a_reference_definition_is() {
    let document = concat!(
        "See [the ADRs](./docs/adr/) and [the site](https://example.invalid/).\n",
        "\n",
        "```markdown\n",
        "[not a link](./sample.md)\n",
        "```\n",
        "\n",
        "[badge]: https://img.example.invalid/b.svg\n",
        "![logo](<./logo.png> \"title\")\n",
    );
    let targets: Vec<String> = markdown_link_targets(document)
        .into_iter()
        .map(|(_, target)| target)
        .collect();
    assert!(
        !targets.iter().any(|target| target == "./sample.md"),
        "a sample inside a fence was read as a link, which is how a rule becomes unlivable: {targets:?}"
    );
    assert!(
        targets.iter().any(|target| target == "./docs/adr/"),
        "the inline link went unread: {targets:?}"
    );
    assert!(
        targets
            .iter()
            .any(|target| target == "https://img.example.invalid/b.svg"),
        "a reference definition went unread, so a README could park every link at the bottom and \
         answer for none of them: {targets:?}"
    );
    assert!(
        targets.iter().any(|target| target == "./logo.png"),
        "an angle-bracketed image target went unread: {targets:?}"
    );
    assert!(followable_from_crates_io("#usage"));
    assert!(!followable_from_crates_io("./CONTRIBUTING.md"));
    assert!(!followable_from_crates_io("docs/adr/0015.md"));
}

#[test]
fn an_include_that_climbs_out_of_its_crate_is_read_and_prose_about_one_is_not() {
    let source = concat!(
        "// keeps the `include_str!` out of normal builds\n",
        "#[doc = include_str!(\"../README.md\")]\n",
        "const CSS: &str = include_str!(\"../theme/x.css\");\n",
        "const SPEC: &str = include_str!( \"../../../spec/x.json\" );\n",
    );
    let found: Vec<String> = included_paths(source)
        .into_iter()
        .map(|(_, path)| path)
        .collect();
    assert_eq!(
        found,
        vec!["../README.md", "../theme/x.css", "../../../spec/x.json"],
        "the include reader either missed a spelling or counted the sentence about one"
    );

    let crate_dir = Path::new("/w/crates/lib");
    let src = crate_dir.join("src");
    assert_eq!(
        lexical_join(&src, "../README.md"),
        crate_dir.join("README.md")
    );
    assert_eq!(
        lexical_join(&src.join("ir"), "../../theme/x.css"),
        crate_dir.join("theme/x.css")
    );
    assert!(
        lexical_join(&src, "../../../spec/x.json")
            .strip_prefix(crate_dir)
            .is_err(),
        "a path climbing above the crate still read as being inside it, which is the one shape \
         the rule exists to reject"
    );

    // And the reader is not blind to a live escape: the workspace still holds
    // one, in the crate where it is legitimate. `publish = false` is what makes
    // it so — an unpublished crate has no package to leave — and the rule above
    // is scoped to published members for exactly that reason, so this is the
    // proof that the scoping is a choice and not a reader that finds nothing.
    let support = "crates/aozora-flavored-markdown-test-support";
    let escapes = included_paths(&read(&format!("{support}/src/lib.rs")));
    assert!(
        escapes.iter().any(|(_, path)| {
            lexical_join(&repo_root().join(support).join("src"), path)
                .strip_prefix(repo_root().join(support))
                .is_err()
        }),
        "no include in {support} reaches outside it any more; either the root README's doctest \
         moved again, or this reader has stopped seeing escapes at all"
    );
}

// ---------------------------------------------------------------------------
// the bump that has to move every version this repo states
// ---------------------------------------------------------------------------
//
// Releasing was three steps in CONTRIBUTING.md performed by hand, and the two
// declarations that turn them into a tool's job — `release.toml` and the
// `[package.metadata.release]` tables — are the same shape as everything else
// in this file: a rule stated in one file and executed by a program somewhere
// else, with no compiler between them. What is different is WHEN the execution
// happens. Every other declaration here is read on every pull request; these
// are read once, on the day somebody cuts a release, against a tree that is
// about to become a tag. A `search` that stopped matching, a version line that
// quietly joined the other one, a generated file the bump does not regenerate
// — each is a thing nobody finds out until the worst moment to find it out.
//
// So the rules below ask the declarations the questions the release itself
// would ask, on every merge instead of once a cycle. Three of them are about
// the shape this workspace is unusual in — two version lines, ADR-0018 — and
// the last two are about the two things `just release` is deliberately NOT
// allowed to do, both of which cargo-release does by default.

/// cargo-release's workspace configuration.
const RELEASE_CONFIG: &str = "release.toml";

/// The per-package half of it.
const RELEASE_TABLE: &str = "package.metadata.release";

/// The group name cargo-release gives a member that inherits
/// `[workspace.package] version`, whatever [`RELEASE_CONFIG`] says its default
/// is. Read it back with `cargo release config --manifest-path <manifest>`.
const INHERITED_GROUP: &str = "workspace";

/// The generated assets `just dist-assets` writes and `just dist-assets-check`
/// compares. The man page in here embeds the CLI's version.
const GENERATED_ASSETS: &str = "dist/assets";

/// The whole of `release.toml`, whose keys sit in no table — so
/// [`manifest_value`] reads them under the empty one.
fn release_config() -> String {
    let path = repo_root().join(RELEASE_CONFIG);
    fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!(
            "reading {RELEASE_CONFIG}: {e}\n\
             It is what `cargo release` reads, and its defaults are the opposite of this repo's \
             policy on every key that matters: publish on, tag on, push on."
        )
    })
}

/// A top-level key of [`RELEASE_CONFIG`].
fn release_setting(key: &str) -> Option<String> {
    manifest_value(&release_config(), "", key).map(str::to_owned)
}

/// The version line a member is on: the literal it declares, or `None` when it
/// inherits `[workspace.package] version`.
fn own_version(manifest: &str) -> Option<String> {
    manifest_value(manifest, "package", "version").map(|value| value.trim_matches('"').to_owned())
}

/// The `shared-version` group a member is bumped in.
///
/// A member inheriting the workspace version is in [`INHERITED_GROUP`] and
/// cannot be moved out of it. A member naming its own version takes its own
/// declaration, and falls back to [`RELEASE_CONFIG`]'s default when it makes
/// none — which is the collapse this file exists to notice.
fn shared_version_group(member: &Member) -> String {
    if own_version(&member.manifest).is_none() {
        return INHERITED_GROUP.to_owned();
    }
    manifest_value(&member.manifest, RELEASE_TABLE, "shared-version")
        .map(|value| value.trim_matches('"').to_owned())
        .or_else(|| {
            release_setting("shared-version").map(|value| value.trim_matches('"').to_owned())
        })
        .unwrap_or_else(|| INHERITED_GROUP.to_owned())
}

#[test]
fn every_version_line_this_workspace_carries_is_bumped_as_one_group() {
    // ADR-0018 consolidated the EPUB generator in on a 0.1.x line of its own
    // and left the parser, its CLI, the wasm bridge and the two dev crates on
    // the workspace's. Nothing in this repo has ever read those two lines as
    // lines. `no_workflow_reads_a_crate_version_out_of_a_manifest_by_hand`
    // names the hazard in its own comment — "this workspace publishes on more
    // than one version line, so a single read of the root manifest names the
    // wrong version for some crate" — and then only forbids a workflow from
    // grepping for one. The publishable-set rules derive everything from
    // `publish = false` and read no version at all.
    //
    // The thing that actually moves those numbers is `cargo release`, and it
    // decides by GROUP: members of one group take one bump, and a member that
    // names its own version and no group falls into whatever default
    // `release.toml` declares. Measured on this workspace, putting the EPUB
    // library in the workspace group takes it from 0.1.0 to 0.6.0 while its
    // CLI, left in `epub`, goes to 0.2.0 — no error, no warning, and the two
    // halves of one crate pair ship as two numbers.
    let members = workspace_members();
    let mut lines: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut undeclared = Vec::new();
    for member in &members {
        let group = shared_version_group(member);
        let Some(version) = own_version(&member.manifest) else {
            lines.entry(group).or_default().insert(workspace_version());
            continue;
        };
        if manifest_value(&member.manifest, RELEASE_TABLE, "shared-version").is_none() {
            undeclared.push(format!("  {} (version = \"{version}\")", member.path));
        }
        lines.entry(group).or_default().insert(version);
    }

    // A rule about two lines says nothing over one.
    let distinct: BTreeSet<&String> = lines.values().flatten().collect();
    assert!(
        distinct.len() >= 2,
        "this workspace came out on {distinct:?} — one version line. Either ADR-0018's second \
         line is gone, in which case retire this rule rather than leaving it passing vacuously, \
         or the reader has stopped telling `version = \"…\"` from `version.workspace = true`."
    );

    assert!(
        undeclared.is_empty(),
        "these members name their own version and no `[{RELEASE_TABLE}] shared-version`:\n{}\n\
         A member with no group takes `{RELEASE_CONFIG}`'s default, which is the group every \
         crate on the workspace line is already in. `cargo release <level>` would then bump it \
         to that line's next number — silently, because a shared version is a thing cargo-release \
         enforces rather than reports. Name the group in the manifest, on every member of the \
         line: it is a property of each of them, and a pair with it on one half only ships as \
         two numbers.",
        undeclared.join("\n")
    );

    for (group, versions) in &lines {
        assert_eq!(
            versions.len(),
            1,
            "the `{group}` group holds {versions:?}. A group is bumped as a unit, so two \
             versions in one is a state the next release resolves by picking one of them."
        );
    }
    assert_eq!(
        lines.len(),
        distinct.len(),
        "{} group(s) for {} version line(s): {lines:?}. Every line needs its own group, or two \
         of them move together the first time somebody runs a bump.",
        lines.len(),
        distinct.len()
    );
}

/// The workspace's own version, the number the `v<version>` tag and the
/// CHANGELOG heading carry.
fn workspace_version() -> String {
    manifest_value(&read("Cargo.toml"), "workspace.package", "version")
        .unwrap_or_else(|| panic!("Cargo.toml has no `[workspace.package] version`"))
        .trim_matches('"')
        .to_owned()
}

#[test]
fn a_second_version_line_with_no_group_of_its_own_reads_as_swept_into_the_first() {
    // The reader, on the two shapes it has to tell apart. This is the
    // configuration DEV-226 specified — one `shared-version` in `release.toml`
    // and nothing in the manifests — and the point is that it looks complete:
    // the key is set, the file exists, `cargo release` runs, and the second
    // version line is gone.
    let epub = Member {
        path: "crates/aozora-flavored-markdown-epub".to_owned(),
        manifest: "[package]\nname = \"e\"\nversion = \"0.1.0\"\n".to_owned(),
    };
    assert_eq!(
        shared_version_group(&epub),
        release_setting("shared-version")
            .map(|value| value.trim_matches('"').to_owned())
            .unwrap_or_default(),
        "a member naming its own version and no group has to read as being in the DEFAULT \
         group; that is what makes the omission visible"
    );

    let grouped = Member {
        path: epub.path,
        manifest: format!(
            "[package]\nname = \"e\"\nversion = \"0.1.0\"\n\n[{RELEASE_TABLE}]\nshared-version = \"epub\"\n"
        ),
    };
    assert_eq!(
        shared_version_group(&grouped),
        "epub",
        "the declaration in the manifest is what settles it"
    );

    let inheriting = Member {
        path: "crates/aozora-flavored-markdown".to_owned(),
        manifest: format!(
            "[package]\nname = \"a\"\nversion.workspace = true\n\n[{RELEASE_TABLE}]\nshared-version = \"epub\"\n"
        ),
    };
    assert_eq!(
        shared_version_group(&inheriting),
        INHERITED_GROUP,
        "a member inheriting `[workspace.package] version` is in the `{INHERITED_GROUP}` group \
         whatever its manifest says — cargo-release decides that, not the file. A reader that \
         believed the manifest here would report a workspace crate as being on the EPUB line."
    );
}

/// One `pre-release-replacements` entry: the edit the release makes to a file
/// that is not a manifest.
struct Replacement {
    member: String,
    file: String,
    search: String,
    exactly: Option<usize>,
}

/// A TOML basic string, unescaped. `search` values carry `\\[` for a literal
/// bracket and `replace` values carry `\n`, and both mean nothing until the
/// escapes are resolved — a reader that matched the raw text would report
/// every search as finding nothing.
fn toml_string(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len());
    let mut chars = literal.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// One quoted field of an inline table.
fn inline_field(chunk: &str, key: &str) -> Option<String> {
    let pattern = Regex::new(&format!(r#"\b{key}\s*=\s*"((?:[^"\\]|\\.)*)""#))
        .unwrap_or_else(|e| panic!("compiling the reader for `{key}`: {e}"));
    pattern
        .captures(chunk)
        .map(|caught| toml_string(&caught[1]))
}

/// Every replacement any member declares, with the member that declares it.
///
/// Read off the manifests rather than listed here for the reason every
/// derivation in this file is: a replacement added to a second package is
/// exactly the state the rule below exists to reject, and a listed set would
/// not contain it.
fn release_replacements() -> Vec<Replacement> {
    let count = Regex::new(r"\bexactly\s*=\s*(\d+)").expect("compiling the `exactly` reader");
    let mut out = Vec::new();
    for member in workspace_members() {
        let mut inside = false;
        for line in member.manifest.lines() {
            let body = line.trim();
            if body.starts_with("pre-release-replacements") {
                inside = true;
                continue;
            }
            if !inside {
                continue;
            }
            if body == "]" {
                inside = false;
                continue;
            }
            if !body.starts_with('{') {
                continue;
            }
            out.push(Replacement {
                member: member.path.clone(),
                file: inline_field(body, "file").unwrap_or_else(|| {
                    panic!("{}'s replacement `{body}` names no `file`", member.path)
                }),
                search: inline_field(body, "search").unwrap_or_else(|| {
                    panic!("{}'s replacement `{body}` names no `search`", member.path)
                }),
                exactly: count
                    .captures(body)
                    .and_then(|caught| caught[1].parse().ok()),
            });
        }
    }
    out
}

#[test]
fn every_replacement_the_release_makes_matches_where_it_says_it_does() {
    // `## [Unreleased]` becoming `## [0.6.0] - <date>` is step 1 of the manual
    // release, and the step most likely to be skipped. As a declaration it is
    // five regular expressions over a file nothing else in this repo compiles
    // a regular expression against — and CHANGELOG.md is the one file here
    // that is APPEND-ONLY by design, so every cycle adds text a search written
    // against last cycle's shape can start matching. `exactly` turns that into
    // an abort, which is the right behaviour and arrives at the wrong moment:
    // mid-release, on a tree that is about to be tagged, after the version
    // step has already rewritten seven manifests.
    //
    // The vale gate reads CHANGELOG.md for retired names and the path gate
    // excuses it as history. Neither compiles these searches. This does, on
    // every merge.
    let declared = release_replacements();
    assert!(
        !declared.is_empty(),
        "no member declares `pre-release-replacements`. Cutting `## [Unreleased]` into a dated \
         section is then a thing somebody has to remember again, and the CHANGELOG heading \
         `just changelog-check` looks for is one nothing writes."
    );

    let mut per_file: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for replacement in &declared {
        per_file
            .entry(replacement.file.as_str())
            .or_default()
            .insert(replacement.member.as_str());

        let path = repo_root()
            .join(&replacement.member)
            .join(&replacement.file);
        let exactly = replacement.exactly.unwrap_or_else(|| {
            panic!(
                "{}'s search `{}` declares no `exactly`. Without it cargo-release accepts any \
                 number of matches, zero included — so a search that has stopped finding its \
                 line reports nothing and the release ships the file unedited.",
                replacement.member, replacement.search
            )
        });
        let found = declared_search_matches(
            &format!(
                "{}'s pre-release-replacement (`file` resolves against the manifest's own \
                 directory, not the workspace root)",
                replacement.member
            ),
            &replacement.search,
            &path,
        );
        assert_eq!(
            found, exactly,
            "`{}` matches {} in {}, and the release declares `exactly = {exactly}`.\n\
             cargo-release aborts on that mismatch — mid-release, after the version step has \
             rewritten every manifest. Fix the search here, where it costs a test.",
            replacement.search, found, replacement.file
        );
    }

    for (file, members) in &per_file {
        assert_eq!(
            members.len(),
            1,
            "{file} is rewritten by {members:?}. cargo-release runs the replacements once per \
             SELECTED package, so a file two members both claim is edited twice in one release \
             — and the second pass reads what the first wrote."
        );
    }
}

#[test]
fn the_search_cargo_release_documents_would_rewrite_this_files_history() {
    // Why those searches are anchored, measured rather than asserted.
    // cargo-release's own documented recipe for this is `search = "Unreleased"`
    // over the whole file. This CHANGELOG holds the word three times: the
    // heading, the link definition — and a sentence in the 0.4.0 section
    // saying "see [Unreleased]" about a splicer that has since shipped. The
    // unanchored form rewrites that sentence into a cross-reference to a
    // release it predates, and `exactly = 1` would abort before it got there.
    let text = read("CHANGELOG.md");
    let documented = text.matches("Unreleased").count();
    assert!(
        documented > 1,
        "`Unreleased` occurs {documented} time(s) in CHANGELOG.md. The anchoring in the \
         replacements is written against a file where it occurs more than once; if that is no \
         longer true, this rule is measuring nothing and the searches can be reconsidered."
    );

    let declared = release_replacements();
    let anchored = declared
        .iter()
        .filter(|replacement| {
            replacement.file.ends_with("CHANGELOG.md") && replacement.search.contains("(?m)")
        })
        .count();
    assert!(
        anchored > 0,
        "no search over CHANGELOG.md is anchored any more, and the word it is looking for \
         appears {documented} times in that file"
    );
}

// ---------------------------------------------------------------------------
// a declared search finds what it says it finds, wherever it is declared
// ---------------------------------------------------------------------------

// How many times a declared search matches the file it names.
//
// One core for every spelling of the same claim — "this text is in that file".
// A manifest writes it as a TOML `search` field and a recipe or a workflow
// writes it as `grep` arguments, and the reason to compile all of them here is
// that nothing else compiles any of them: what is being searched is prose or a
// manifest, and neither is type-checked.
fn declared_search_matches(site: &str, pattern: &str, path: &Path) -> usize {
    let text = fs::read_to_string(path).unwrap_or_else(|e| {
        panic!(
            "{site} searches {} and it does not read: {e}",
            path.display()
        )
    });
    let compiled = Regex::new(pattern)
        .unwrap_or_else(|e| panic!("{site}'s search `{pattern}` is not a regular expression: {e}"));
    compiled.find_iter(&text).count()
}

// The words a shell would see, with quoted runs kept whole.
//
// `shell_tokens` flattens quotes into separators, which is the right reading
// for finding a sub-command and the wrong one here: `grep -oE 'rev =
// "[0-9a-f]{40}"' Cargo.toml` is a three-argument call whose pattern holds
// both a space and a quote. `$(` reopens an unquoted context the way the shell
// does, so a substitution inside a double-quoted assignment reads as the
// command it is.
fn quoted_words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    let mut quote = None;
    let mut chars = line.chars().peekable();
    while let Some(ch) = chars.next() {
        // A command substitution starts a fresh quoting context even inside a
        // double-quoted word, so it ends the word AND the quote:
        // `have="v$(grep -oE '…' file)"` is a call, not a string.
        let substitution = ch == '$' && chars.peek() == Some(&'(');
        if substitution {
            chars.next();
        }
        if substitution || (quote.is_none() && ch.is_whitespace()) {
            if quoted || !word.is_empty() {
                out.push(mem::take(&mut word));
            }
            quoted = false;
            quote = None;
        } else if quote == Some(ch) {
            quote = None;
        } else if quote.is_none() && (ch == '\'' || ch == '"') {
            quote = Some(ch);
            quoted = true;
        } else {
            word.push(ch);
        }
    }
    if quoted || !word.is_empty() {
        out.push(word);
    }
    out
}

// The searches a shell script runs to READ A VALUE out of a named file.
//
// `grep -o` prints the matched text and nothing else, so it is only ever
// written where the match IS the answer: a pinned version out of a `Dockerfile`
// ARG, a tool's own version out of `dist-workspace.toml`. `-q` is the other
// shape — it asks a yes/no question and "no" is one of the two answers it is
// entitled to give — so a `-q` search is not read here.
//
// What a `-o` search cannot do is match nothing; the failure message below
// carries what that cost.
fn value_searches(text: &str) -> Vec<(String, String)> {
    // Flags that swallow the word after them, so that word is not mistaken for
    // the pattern.
    const TAKES_A_VALUE: &[&str] = &["-A", "-B", "-C", "-m", "-e", "-f", "-d"];
    let mut out = Vec::new();
    for line in text.lines() {
        let words = quoted_words(line);
        for (at, word) in words.iter().enumerate() {
            if word.rsplit(['|', ';', '&', '`']).next() != Some("grep") {
                continue;
            }
            let mut rest = words[at + 1..].iter();
            let mut prints_the_match = false;
            let mut pattern = None;
            while let Some(word) = rest.next() {
                if let Some(flags) = word.strip_prefix('-') {
                    prints_the_match |= flags.contains('o');
                    if TAKES_A_VALUE.contains(&word.as_str()) {
                        rest.next();
                    }
                    continue;
                }
                pattern = Some(word.clone());
                break;
            }
            let (Some(pattern), Some(file)) = (pattern, rest.next()) else {
                continue;
            };
            // A pattern or a path assembled at run time is not a literal this
            // reader can resolve, and a `grep` with no path reads a pipe.
            let resolved = |value: &str| !value.contains('$') && !value.contains('*');
            if !prints_the_match
                || !resolved(&pattern)
                || !resolved(file)
                || file.starts_with(['|', '>', '<', '&', ';', ')'])
            {
                continue;
            }
            out.push((pattern, file.clone()));
        }
    }
    out
}

#[test]
fn every_search_this_repo_runs_to_read_a_value_finds_one() {
    let mut sites: Vec<(String, String, String)> = Vec::new();
    for (recipe, line) in expanded_recipe_lines(&read("Justfile")) {
        for (pattern, file) in value_searches(&line) {
            sites.push((format!("the `{recipe}` recipe"), pattern, file));
        }
    }
    for path in workflow_files() {
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let text =
            fs::read_to_string(&path).unwrap_or_else(|e| panic!("reading {}: {e}", path.display()));
        for (pattern, file) in value_searches(&text) {
            sites.push((name.clone(), pattern, file));
        }
    }

    assert!(
        sites.len() >= 4,
        "only {} value-reading search(es) found across the Justfile and the workflows; the \
         reader is not finding `grep -o` any more, and a rule that finds nothing passes on \
         everything: {sites:?}",
        sites.len()
    );

    for (site, pattern, file) in &sites {
        let found = declared_search_matches(site, pattern, &repo_root().join(file));
        assert!(
            found > 0,
            "{site} reads a value out of {file} with `{pattern}`, and that pattern matches \
             nothing in it. The variable comes back empty and the script runs its \"absent\" \
             branch on every tree — which is how `just doctor` came to fail unconditionally \
             while the gate that owns the same question was passing."
        );
    }
}

/// The `just` recipes any member's `pre-release-hook` runs, with the member.
fn release_hooks() -> Vec<(String, String)> {
    let mut out = Vec::new();
    for member in workspace_members() {
        let Some(value) = manifest_value(&member.manifest, RELEASE_TABLE, "pre-release-hook")
        else {
            continue;
        };
        let command = quoted_items(value);
        let Some(at) = command.iter().position(|word| word == "just") else {
            continue;
        };
        if let Some(recipe) = command.get(at + 1) {
            out.push((member.path, recipe.clone()));
        }
    }
    out
}

#[test]
fn every_generated_file_a_gate_compares_is_regenerated_when_the_version_moves() {
    // Step 2 of the manual release, and the one with a gate already pointed at
    // it. `dist/assets/man/aozora-flavored-markdown.1` is generated from the
    // CLI's clap definition and embeds the version twice;
    // `just dist-assets-check` regenerates it and diffs. Both halves were
    // already here — the gate has been in `[group('gate')]` and the recipe
    // that writes it in `[group('release')]` — and nothing connected them to
    // the thing that changes the version. So the release commit, the one
    // commit in the cycle that MUST be green, was the commit that failed a
    // gate, and the way you found out was by pushing it.
    //
    // The rule is over the SET: any generated asset that comes to carry the
    // version is covered, not the man page by name.
    let justfile = read("Justfile");
    let version = workspace_version();
    let generated = git_tracked(&[GENERATED_ASSETS.to_owned()]);
    assert!(
        !generated.is_empty(),
        "`git ls-files {GENERATED_ASSETS}` lists nothing; the reader is not finding the assets \
         `just dist-assets` writes"
    );

    let carriers: Vec<&String> = generated
        .iter()
        .filter(|path| read(path).contains(&version))
        .collect();
    assert!(
        !carriers.is_empty(),
        "no file under {GENERATED_ASSETS} carries the workspace version {version} any more. \
         That is the whole reason the bump has to regenerate them; if it has stopped being \
         true, say so here rather than leaving the hook unexplained."
    );

    let hooks = release_hooks();
    assert!(
        !hooks.is_empty(),
        "{carriers:?} carry version {version} and no member declares a \
         `[{RELEASE_TABLE}] pre-release-hook`. Those files are regenerated by a recipe a \
         `[group('gate')]` check runs, so a bump with no hook leaves the release commit failing \
         a gate — found on push, on the commit that is about to become a tag."
    );

    // Derived, not named: the hook has to run the WRITING half of a pair whose
    // checking half is a gate. That is what makes "the release commit stays
    // green" the thing being asserted, rather than "a hook exists".
    let gates = recipes_in_group(&justfile, "gate");
    let mut regenerates_a_gated_file = false;
    for (member, recipe) in &hooks {
        assert!(
            recipe_exists(&justfile, recipe),
            "{member}'s pre-release-hook runs `just {recipe}` and the Justfile has no such \
             recipe. cargo-release would fail the release on it."
        );
        regenerates_a_gated_file |= gates.contains(&format!("{recipe}-check"));
    }
    assert!(
        regenerates_a_gated_file,
        "no pre-release-hook runs a recipe whose `-check` half is a gate: {hooks:?}. A hook that \
         regenerates nothing any gate compares is not what keeps the release commit green."
    );
}

/// The cargo-release steps `just release` is allowed to run: the three that
/// write files in the work tree. What is missing from it is the point —
/// `commit`, `tag`, `push` and `publish` are the four that leave it.
const FILE_WRITING_STEPS: &[&str] = &["version", "replace", "hook", "config"];

#[test]
fn no_release_this_repo_can_run_uploads_to_crates_io_itself() {
    // The fifth reading of `publish = false`, and the first of a file that is
    // not a cargo manifest. Everything in "the ladder that reaches crates.io"
    // above derives the uploaded SET from the manifests and then checks that
    // `publish-crates.yml` reaches all of it — on the premise that the
    // workflow is the only thing that uploads. ADR-0015 is explicit about why
    // that matters: the upload runs behind the `release` GitHub Environment,
    // with a short-lived OIDC token that exists only after a required-reviewer
    // approval.
    //
    // cargo-release publishes BY DEFAULT, and this image has had it installed
    // since tier D was added — reachable from `just shell`, from a laptop,
    // with no approval in front of it and a long-lived token if one is in the
    // developer's cargo credentials. No rule above could see that, because
    // every one of them reads `cargo publish` and this is not one.
    let image = image_tools(&read("Dockerfile"));
    assert!(
        image.contains("cargo-release"),
        "the image no longer installs cargo-release; if the tool is gone, so is this rule's \
         subject — but so is `just release`, so check that first"
    );

    assert_eq!(
        release_setting("publish").as_deref(),
        Some("false"),
        "{RELEASE_CONFIG} does not turn cargo-release's publishing off. Its default is ON, and \
         a `cargo release` run from `just shell` would then upload straight to crates.io: no \
         environment approval, no OIDC token, no preflight, and an upload is the one operation \
         crates.io will not let anybody take back."
    );
    for member in workspace_members() {
        assert_ne!(
            manifest_value(&member.manifest, RELEASE_TABLE, "publish"),
            Some("true"),
            "{} turns publishing back on for itself. A per-package key overrides \
             {RELEASE_CONFIG}, so one manifest is all it takes to put a second uploader beside \
             the approved one.",
            member.path
        );
    }
}

#[test]
fn the_release_recipe_runs_only_the_steps_that_write_files() {
    // The other default, and the one that fails by SUCCEEDING. Commits and
    // tags here are SSH-signed and signing is mandatory; the key is on the
    // host, and docker-compose mounts the work tree and the cargo caches and
    // nothing else (ADR-0002). A `git commit` inside the dev image therefore
    // does not fail for want of a key — it succeeds, unsigned. So the recipe
    // runs cargo-release's file-writing steps and stops, and the release
    // commit and the `v<version>` tag are made afterwards, on the host.
    //
    // "Stops" is a property of an argument list, which is a thing that gets
    // shortened. A bare `cargo release <level>` runs every step there is, and
    // it is both the obvious simplification of this recipe and the spelling
    // every cargo-release tutorial shows.
    let justfile = read("Justfile");
    let invocation = Regex::new(r"cargo\s+release\s+(\S+)").expect("compiling the reader");
    let mut steps: Vec<(String, String)> = Vec::new();
    for (recipe, line) in expanded_recipe_lines(&justfile) {
        for caught in invocation.captures_iter(&line) {
            steps.push((recipe.clone(), caught[1].to_owned()));
        }
    }
    assert!(
        !steps.is_empty(),
        "no recipe runs `cargo release`. The tool is installed in the dev image and the \
         workspace carries a {RELEASE_CONFIG} for it; a configuration nothing executes is the \
         shape this whole file exists to reject."
    );

    for (recipe, step) in &steps {
        assert!(
            FILE_WRITING_STEPS.contains(&step.as_str()),
            "`just {recipe}` runs `cargo release {step}`.\n\
             Only {FILE_WRITING_STEPS:?} write inside the work tree and stop there. `commit`, \
             `tag` and `push` reach git — and a commit made in the dev image is not a commit \
             that fails for want of the signing key, it is an UNSIGNED commit that succeeds. A \
             bare `cargo release <level>` runs all of them.",
        );
        assert!(
            recipe_runs_in_a_container(&justfile, recipe),
            "`just {recipe}` runs `cargo release {step}` outside the dev image. Execution here \
             is docker-only (ADR-0002); a recipe that reaches the host toolchain also reaches \
             the host's git configuration, which is the one thing this split is arranged around."
        );
    }

    for key in ["tag", "push"] {
        assert_eq!(
            release_setting(key).as_deref(),
            Some("false"),
            "{RELEASE_CONFIG} leaves `{key}` at cargo-release's default, which is on. The \
             recipe not running those steps is the first half; this is the half that holds when \
             somebody runs the tool by hand from `just shell`. The tag name matters too: \
             cargo-release spells it `<crate>-v<version>` for a package in a subdirectory, and \
             `v<version>` is what cliff.toml's tag_pattern, the semver gate's baseline and \
             release.yml's trigger all expect."
        );
    }
}
