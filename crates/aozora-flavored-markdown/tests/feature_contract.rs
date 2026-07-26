//! Every Cargo feature this workspace declares must do something.
//!
//! A feature is published API: a consumer writes it into their manifest, and
//! removing it is a breaking release. A feature that gates nothing is
//! therefore the worst of both — it constrains this crate and buys the
//! consumer nothing, while reading in the manifest as a real choice. That is
//! exactly what `html` was: `default = ["html"]` with zero
//! `#[cfg(feature = "html")]` anywhere, so "build without the renderer"
//! looked supported and was not (`ir::Block::Aozora` carries rendered HTML,
//! so it never could be).
//!
//! Nothing else in the workspace could see it. `cargo-shear` and
//! `cargo-udeps` find unused *dependencies*; clippy and rustc never see the
//! manifest; `--all-features` and the default build both compile the same
//! code, so no test could tell the two apart. The gate that was missing is a
//! reader of the manifest, and it is written as a rule over every feature of
//! every crate rather than as a note about the one that was dead.
//!
//! A feature is live when it enables something (another feature, or an
//! optional dependency via `dep:`) or when its own crate's source reads it in
//! a `cfg`. `[features]` is parsed by hand rather than with a TOML crate: the
//! grammar in play is `name = [ … ]`, and a dev-dependency added to read it
//! would itself be a dependency edge this workspace's gates then police.

use core::mem;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

/// The features that exist today, as `crate/feature`. Named so this cannot
/// pass by parsing nothing — the failure mode of every hand-written reader.
const KNOWN_FEATURES: &[&str] = &[
    "aozora-flavored-markdown/miette",
    "aozora-flavored-markdown/theme",
    "aozora-flavored-markdown/tsify",
    "aozora-flavored-markdown-wasm/default",
    "aozora-flavored-markdown-wasm/panic-hook",
];

/// The workspace root, reached from this crate's manifest directory. The
/// contract is workspace-wide because a dead feature in the wasm or CLI crate
/// is the same defect; scoping this to one crate would have let the next one
/// through.
fn workspace_root() -> PathBuf {
    let mut root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    root.pop();
    root.pop();
    root
}

/// One declared feature and what it enables.
#[derive(Debug)]
struct Feature {
    krate: String,
    name: String,
    enables: Vec<String>,
}

/// Every `<crate>/Cargo.toml` in the workspace, by directory name.
fn member_manifests() -> Vec<(String, PathBuf)> {
    let crates = workspace_root().join("crates");
    let mut out = Vec::new();
    for entry in fs::read_dir(&crates).expect("crates/ must be readable") {
        let path = entry.expect("a directory entry must be readable").path();
        let manifest = path.join("Cargo.toml");
        if manifest.is_file() {
            let name = path
                .file_name()
                .expect("a crate directory has a name")
                .to_string_lossy()
                .into_owned();
            out.push((name, manifest));
        }
    }
    out.sort();
    assert!(!out.is_empty(), "no crate manifest found under crates/");
    out
}

/// The `[features]` table of one manifest.
///
/// Comments and blank lines are dropped, an array wrapped over several lines
/// is joined, and the table ends at the next `[section]`.
fn features_in(krate: &str, manifest: &str) -> Vec<Feature> {
    let mut out = Vec::new();
    let mut in_table = false;
    let mut pending = String::new();
    for line in manifest.lines() {
        let code = line.split_once('#').map_or(line, |(before, _)| before);
        let trimmed = code.trim();
        if trimmed.is_empty() {
            continue;
        }
        if pending.is_empty() && trimmed.starts_with('[') {
            in_table = trimmed == "[features]";
            continue;
        }
        if !in_table {
            continue;
        }
        pending.push_str(trimmed);
        if !pending.contains(']') {
            continue;
        }
        let entry = mem::take(&mut pending);
        let (name, value) = entry
            .split_once('=')
            .expect("a feature entry is `name = [ … ]`");
        out.push(Feature {
            krate: krate.to_owned(),
            name: name.trim().trim_matches('"').to_owned(),
            enables: value
                .trim()
                .trim_start_matches('[')
                .trim_end_matches(']')
                .split(',')
                .map(|item| item.trim().trim_matches('"').to_owned())
                .filter(|item| !item.is_empty())
                .collect(),
        });
    }
    out
}

/// Whether one crate's own source reads `feature` in a `cfg`.
fn source_gates_on(root: &Path, feature: &str) -> bool {
    let needle = format!("feature = \"{feature}\"");
    let mut sources = Vec::new();
    let src = root.join("src");
    if src.is_dir() {
        collect_rust_sources(&src, &mut sources);
    }
    sources
        .iter()
        .any(|path| fs::read_to_string(path).is_ok_and(|text| text.contains(&needle)))
}

fn collect_rust_sources(dir: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(dir).expect("src/ must be readable") {
        let path = entry.expect("a directory entry must be readable").path();
        if path.is_dir() {
            collect_rust_sources(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

fn declared_features() -> Vec<Feature> {
    member_manifests()
        .into_iter()
        .flat_map(|(krate, manifest)| {
            let text = fs::read_to_string(&manifest).expect("a manifest must be readable");
            features_in(&krate, &text)
        })
        .collect()
}

#[test]
fn every_declared_feature_gates_something() {
    let root = workspace_root().join("crates");
    let mut dead: Vec<String> = Vec::new();
    for feature in declared_features() {
        if !feature.enables.is_empty() {
            continue;
        }
        if source_gates_on(&root.join(&feature.krate), &feature.name) {
            continue;
        }
        dead.push(format!("{}/{}", feature.krate, feature.name));
    }
    assert!(
        dead.is_empty(),
        "these features enable nothing and no `cfg` reads them, so a consumer who writes \
         one into their manifest gets a different manifest and the same build: {dead:?}"
    );
}

#[test]
fn the_reader_finds_the_features_the_workspace_declares() {
    // The anti-vacuity half: a parser that quietly matched nothing would make
    // the rule above pass for a workspace full of dead features.
    let found: BTreeSet<String> = declared_features()
        .iter()
        .map(|f| format!("{}/{}", f.krate, f.name))
        .collect();
    let expected: BTreeSet<String> = KNOWN_FEATURES.iter().map(|f| (*f).to_owned()).collect();
    assert_eq!(
        found, expected,
        "the declared feature set changed; a new feature is a published promise, so add it \
         here deliberately (and a retired one must leave)"
    );
}

#[test]
fn the_rule_reads_a_features_table_the_way_cargo_does() {
    // The reader itself, on the table this crate used to ship plus the two
    // shapes it must not trip on: a wrapped array and a trailing comment.
    let manifest = "\
[package]
name = \"x\"

[features]
default = [\"html\"]
# HTML renderer.
html = []
theme = []
tsify = [\n\"dep:tsify\",\n\"dep:wasm-bindgen\",\n] # bindings
";
    let features = features_in("x", manifest);
    let names: Vec<&str> = features.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["default", "html", "theme", "tsify"]);
    assert_eq!(features[0].enables, ["html"]);
    assert!(
        features[1].enables.is_empty(),
        "`html = []` enabled nothing"
    );
    assert_eq!(features[3].enables, ["dep:tsify", "dep:wasm-bindgen"]);
}

// ---------------------------------------------------------------------------
// what a feature drags in — the renderer must stay out of the libraries
// ---------------------------------------------------------------------------
//
// A feature is a published promise, and so is its cost. The `miette` feature
// promises `impl miette::Diagnostic` and nothing else: the trait is a handful
// of methods, while miette's `fancy` renderer pulls `owo-colors`,
// `supports-color`, `supports-hyperlinks`, `terminal_size`, `textwrap` and a
// backtrace crate. Cargo features are additive and unify across a build, so a
// library that enables `fancy` spends every consumer's dependency budget on a
// terminal renderer that consumer may never call — and cannot opt out of.
//
// Nothing else in the workspace can see this. `cargo-shear` and `cargo-udeps`
// find unused dependencies, not over-featured ones; `cargo-deny` reads
// licences and advisories; clippy and rustc never see a manifest. The gate
// that was missing is a reader of the dependency tables, and the trap it
// closes is specific: workspace inheritance *unions* features, so a member
// can only ever widen `[workspace.dependencies]`. While that entry carried
// `features = ["fancy"]`, no member could take the trait alone — the library
// would have inherited the renderer whatever it wrote.

/// The raw value of one dependency entry — `miette` under `[dependencies]`
/// gives `{ workspace = true, features = ["fancy"] }`.
///
/// Single-line entries only, which is every one this workspace writes. A
/// wrapped one yields the first line and would read as naming no feature, so
/// each caller below asserts on an entry it names rather than on the sweep
/// alone.
fn dependency_entry(manifest: &str, table: &str, name: &str) -> Option<String> {
    let mut in_table = false;
    for line in manifest.lines() {
        let code = line.split_once('#').map_or(line, |(before, _)| before);
        let trimmed = code.trim();
        if trimmed.starts_with('[') && trimmed.ends_with(']') {
            in_table = trimmed == table;
            continue;
        }
        let Some((key, value)) = trimmed.split_once('=') else {
            continue;
        };
        if in_table && key.trim().trim_matches('"') == name {
            return Some(value.trim().to_owned());
        }
    }
    None
}

fn manifest_text(path: &Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()))
}

/// Whether a dependency entry turns miette's graphical renderer on.
fn enables_the_renderer(entry: &str) -> bool {
    entry.contains("\"fancy\"")
}

/// The crates that may.
///
/// A binary chooses a renderer because a binary is what prints; the two CLIs
/// install miette's graphical handler on startup, so `fancy` is theirs by
/// right.
///
/// `aozora-flavored-markdown-epub` is on this list and does not belong on it:
/// it is a published library with no binary of its own, it uses miette only
/// for `#[derive(Diagnostic)]` on its error type, and its `fancy` is a
/// leftover of the workspace entry that used to force it on everyone. Named
/// here rather than left invisible — dropping it is a change to the dependency
/// graph, which is not this rule's to make.
const MAY_ENABLE_THE_RENDERER: &[&str] = &[
    "aozora-flavored-markdown-cli",
    "aozora-flavored-markdown-epub-cli",
    "aozora-flavored-markdown-epub",
];

#[test]
fn the_workspace_miette_entry_stays_narrow_enough_for_a_library_to_inherit() {
    let entry = dependency_entry(
        &manifest_text(&workspace_root().join("Cargo.toml")),
        "[workspace.dependencies]",
        "miette",
    )
    .expect("the workspace must declare `miette`; retarget this rule rather than deleting it");
    assert!(
        entry.contains("default-features = false"),
        "the inherited `miette` entry must be feature-minimal: inheritance unions features, so \
         whatever this entry turns on, no member can turn back off. Found: {entry}"
    );
    assert!(
        !entry.contains('['),
        "the inherited `miette` entry names a feature, which every member then carries whether \
         it reports with miette or merely implements the trait. Move it down to the members \
         that render. Found: {entry}"
    );
}

#[test]
fn the_library_takes_the_trait_and_the_cli_takes_the_renderer() {
    let crates = workspace_root().join("crates");
    let library = dependency_entry(
        &manifest_text(&crates.join("aozora-flavored-markdown/Cargo.toml")),
        "[dependencies]",
        "miette",
    )
    .expect("the library must declare `miette` behind its feature; retarget rather than delete");
    assert!(
        library.contains("optional = true"),
        "`miette` must stay optional, or the non-default feature that gates it is a fiction: \
         {library}"
    );
    assert!(
        !library.contains('['),
        "the library must name no miette feature — it implements the trait and renders nothing. \
         Found: {library}"
    );

    // The CLI is what turns the feature on, and it is also what keeps the
    // library's own `#[cfg(feature = "miette")]` unit tests inside the
    // workspace build `just test` runs — feature resolution is per package
    // across the whole build, so the CLI's choice is what compiles them.
    // Dropping it here would take those tests out of every gate at once,
    // without failing one.
    let cli = dependency_entry(
        &manifest_text(&crates.join("aozora-flavored-markdown-cli/Cargo.toml")),
        "[dependencies]",
        "aozora-flavored-markdown",
    )
    .expect("the CLI must depend on the library; retarget rather than delete");
    assert!(
        cli.contains("\"miette\""),
        "the CLI must enable the library's `miette` feature: it reports through miette, and a \
         non-default feature no member enables is compiled by no gate. Found: {cli}"
    );
}

#[test]
fn only_a_crate_that_prints_turns_miettes_graphical_renderer_on() {
    let enabling: Vec<String> = member_manifests()
        .into_iter()
        .filter(|(_, manifest)| {
            dependency_entry(&manifest_text(manifest), "[dependencies]", "miette")
                .is_some_and(|entry| enables_the_renderer(&entry))
        })
        .map(|(krate, _)| krate)
        .collect();
    assert!(
        !enabling.is_empty(),
        "no member names `fancy` at all; the reader must be retargeted, not deleted — this is \
         also what the workspace entry looked like when it forced the renderer on everyone"
    );
    let unexpected: Vec<&String> = enabling
        .iter()
        .filter(|krate| !MAY_ENABLE_THE_RENDERER.contains(&krate.as_str()))
        .collect();
    assert!(
        unexpected.is_empty(),
        "{unexpected:?} enable miette's `fancy` renderer. Features unify across a build, so a \
         library that does spends its consumers' dependency budget on a terminal renderer they \
         cannot opt out of"
    );
}

#[test]
fn the_dependency_reader_scopes_a_table_the_way_cargo_does() {
    // The reader itself, on the two shapes it must not confuse: a key of the
    // same name in another table, and the `[features]` entry that shares the
    // dependency's name because it gates it.
    let manifest = "\
[dependencies]
miette = { workspace = true, optional = true }

[dev-dependencies]
miette = { workspace = true, features = [\"fancy\"] }

[features]
miette = [\"dep:miette\"]
";
    assert_eq!(
        dependency_entry(manifest, "[dependencies]", "miette").as_deref(),
        Some("{ workspace = true, optional = true }"),
        "a table must be read to its own end"
    );
    assert!(
        !enables_the_renderer(&dependency_entry(manifest, "[dependencies]", "miette").unwrap()),
        "the `fancy` two tables down is not this entry's"
    );
    assert_eq!(
        dependency_entry(manifest, "[dependencies]", "serde"),
        None,
        "an absent dependency must read as absent, not as the next entry"
    );
}

#[test]
fn a_feature_is_live_when_a_cfg_reads_it() {
    // `theme` enables nothing either — it is live only because the source
    // gates on it, which is the branch that tells it apart from `html`.
    let crates = workspace_root().join("crates");
    let library = crates.join("aozora-flavored-markdown");
    assert!(
        source_gates_on(&library, "theme"),
        "`theme` must stay a `cfg`-read feature, or the liveness branch goes untested"
    );
    assert!(
        !source_gates_on(&library, "html"),
        "a `#[cfg(feature = \"html\")]` is back; the feature it names cannot exist"
    );
}
