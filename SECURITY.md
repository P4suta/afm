# Security policy

## Reporting a vulnerability

If you discover a security vulnerability in aozora-flavored-markdown — a parser crash on
untrusted input, a memory-safety issue, an HTML-injection bypass, or
anything with exploitative potential — **do not open a public
issue**. Instead:

1. Preferred: open a private report via
   [GitHub Security Advisories](https://github.com/P4suta/aozora-flavored-markdown/security/advisories/new).
   This lets us discuss and patch before disclosure.
2. Alternative: email the maintainer at
   `42543015+P4suta@users.noreply.github.com` with the subject
   `[aozora-flavored-markdown security] <short summary>`.

Please include:

- The shortest input or reproduction steps that trigger the issue.
- The aozora-flavored-markdown version / commit hash and the Rust toolchain version.
- Whether the issue is reachable via untrusted input (e.g. rendering
  user-supplied markdown).
- Your proposed CVSS severity, if you have one in mind.

## Response expectations

- We acknowledge reports within **7 days**.
- Triage, patch, and coordinated disclosure typically complete within
  **30–60 days** for high-severity issues, faster for critical ones.
- Credits (unless you prefer anonymity) are noted in `CHANGELOG.md`
  and the security advisory.

## Scope

In scope:
- Crashes, panics, or non-termination on any UTF-8 or Shift_JIS input
  within 10 MiB.
- HTML-escape bypass in the renderer (the splice surface in
  `crates/aozora-flavored-markdown/src/ast_splice.rs`, the fragment
  renderer in `src/fragment.rs`, and the sibling parser's own HTML
  output), since rendered output is embedded in web pages.
- Internal PUA sentinel (`U+E001..=U+E004`) leaking into rendered HTML.
  `src/constructs.rs` replaces each Aozora construct with a sentinel
  before comrak parses; the splicer must resolve every sentinel —
  including those that comrak routes into literal contexts (inline code
  spans, link/image destinations), where the notation renders as its
  original source rather than interpreted HTML. A surviving sentinel (or
  a desynced construct cursor that swaps one notation's content for
  another's) is a bug, gated by the `check_no_sentinel_leak` invariant.
- CommonMark / GFM conformance regressions that enable a bypass.
- Integer overflow, out-of-bounds reads (we `#![forbid(unsafe_code)]`
  in our own crates; `upstream/comrak/` is unsafe-free too).

Out of scope:
- Bugs in vendored `upstream/comrak/` that also reproduce against
  pristine comrak at the same tag — please report those upstream at
  <https://github.com/kivikakk/comrak>.
- Denial-of-service via inputs that simply take a long time to parse
  without panicking (we track these as perf issues, not vulns).
- Issues in dependencies with no aozora-md-specific exploitation path —
  cargo-deny's advisory check catches these at CI time.

## Vendored comrak advisory tracking

comrak is **vendored as a path dependency** at `upstream/comrak/`
(pinned bit-for-bit to upstream v0.52.0; version in
`upstream/comrak/Cargo.toml`, commit in `upstream/comrak/COMRAK_SHA`;
ADR-0001 keeps the diff at 0 lines). A path dependency does **not**
appear in the registry dependency graph that `cargo audit` and
`cargo deny` walk, so neither tool would flag a [RustSec][rustsec]
advisory filed against the `comrak` crate at our pinned version. That
is a real supply-chain blind spot for a vendored fork.

We close it with a dedicated per-PR gate, `just audit-comrak` (wired
into `just audit` → `just ci`, and run as its own leg in the `audit`
matrix of `.github/workflows/ci.yml`). The gate synthesises a one-crate
`Cargo.lock` pinning `comrak` at the vendored version as a crates.io
package and runs the authoritative `cargo audit` engine against it, so
RustSec advisory version-range matching applies to the vendored tree
exactly as it would to a registry dependency. There is **no scheduled
/ cron workflow** (maintainer preference); the check rides every pull
request instead.

When the gate fails it prints the matching `RUSTSEC-…` id. The fix is
normally to advance the vendored tree past the patched version with
`just upstream-sync <tag>`. If — and only if — an advisory provably
does not apply to how aozora-flavored-markdown drives comrak (aozora-flavored-markdown composes vanilla comrak as
a black box and never enables its raw-HTML passthrough by default), the
advisory id may be recorded as a documented `ignore` in the
`audit-comrak` recipe with a one-line justification, mirroring the
`[advisories] ignore` convention in `deny.toml`.

[rustsec]: https://rustsec.org/

## Release profile: `panic = "abort"`

The release profile builds with `panic = "abort"`. A panic that is
nevertheless reached at runtime therefore **aborts the entire host
process** (`SIGABRT`) — it does not unwind and cannot be caught with
`std::panic::catch_unwind`. aozora-flavored-markdown targets a panic-free rendering path on
untrusted input (enforced by the fuzz harnesses and the no-bare-`［＃`
Tier-A invariant), but an embedder must treat any residual panic as a
hard crash of its own process. **Pre-validate untrusted input** (cap
length — the security scope above is bounded at 10 MiB — reject inputs
you will not render) before calling into aozora-flavored-markdown in a process whose
liveness matters, and isolate rendering of attacker-controlled content
in a worker/subprocess if a single render must not be able to take the
host down. Report any panic reachable from a well-formed call as a
vulnerability per the policy above.

## Supported versions

aozora-flavored-markdown is pre-1.0. Only the `main` branch is supported; security fixes
land there and in the next tagged release.

| Version | Supported |
|---|---|
| main  | ✅ |
| <1.0  | ❌ (use main) |
