# Security policy

## Reporting a vulnerability

**Do not open a public issue.** Open a private report via
[GitHub Security Advisories](https://github.com/P4suta/aozora-flavored-markdown/security/advisories/new),
or email `42543015+P4suta@users.noreply.github.com` with the subject
`[aozora-flavored-markdown security] <short summary>`.

Include the shortest input that reproduces, the version or commit, and
whether the issue is reachable from untrusted input. We acknowledge within
7 days; high-severity issues are usually patched and disclosed within
30–60 days. Credit goes in `CHANGELOG.md` and the advisory unless you
prefer anonymity.

## Scope

In scope:

- Crashes, panics or non-termination on any UTF-8 or Shift_JIS input within
  10 MiB.
- HTML-escape bypass anywhere in the render path — output is embedded in
  web pages.
- An internal PUA sentinel (`U+E000..=U+E004`) reaching the rendered HTML,
  or a desynced construct cursor swapping one notation's content for
  another's. Gated by the `check_no_sentinel_leak` invariant.
- CommonMark / GFM conformance regressions that enable a bypass.
- Integer overflow or out-of-bounds reads. Every crate here is
  `#![forbid(unsafe_code)]`.

Out of scope:

- Bugs that reproduce against comrak on its own at the version this
  workspace pins — report those at <https://github.com/kivikakk/comrak>.
- Slow-but-terminating inputs. Those are perf issues.
- Dependency advisories with no exploitation path here; cargo-deny catches
  them at CI time.

## Advisory tracking

`just deny` checks the shipped workspace and the fuzz workspace on every pull
request, and `.github/workflows/audit.yml` repeats the same cargo-deny scan
weekly. The scheduled scan covers advisories published against a lockfile
nobody has changed.

An `unmaintained`, `unsound` or `yanked` crate fails the scan too, not only a
live vulnerability.

On a hit the gate prints the matching `RUSTSEC-…` id, and the fix is
normally a version bump. An advisory that provably does not apply to how
this crate drives the dependency — comrak is driven as a black box, with
raw-HTML passthrough off by default — may instead be recorded as a
documented `ignore` in `deny.toml`.

## Release profile: `panic = "abort"`

A panic reached at runtime **aborts the host process**; it does not unwind
and `catch_unwind` cannot see it. The rendering path is panic-free on
untrusted input (enforced by the fuzz harnesses and the Tier-A invariant),
but an embedder must treat any residual panic as a hard crash of its own
process: pre-validate untrusted input and cap its length, and isolate
attacker-controlled renders in a worker or subprocess where liveness
matters. A panic reachable from a well-formed call is a vulnerability under
the policy above.

## Supported versions

Pre-1.0: only `main` is supported. Security fixes land there and in the
next tagged release.
