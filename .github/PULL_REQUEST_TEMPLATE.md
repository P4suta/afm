## Summary

<!-- One or two sentences: what this PR changes and why. -->

## Type of change

- [ ] Bug fix
- [ ] New feature (CLI flag, public API surface, IR variant, …)
- [ ] Refactor (no behaviour change)
- [ ] Documentation / book / ADR
- [ ] CI / developer tooling
- [ ] Bumping a pinned dependency version (`aozora`, `comrak`, …)

## Checklist

- [ ] `just ci` passes locally (lint + build + test + spec-* + coverage
      + playground-build).
- [ ] Added or updated tests that exercise the change.
- [ ] Updated `CHANGELOG.md` under `[Unreleased]` (or stated why it
      doesn't need a changelog entry).
- [ ] Commit messages follow Conventional Commits (lefthook enforces).
- [ ] If this adds a new 青空文庫 notation: filed it in the sibling
      [`P4suta/aozora`](https://github.com/P4suta/aozora) repo first
      (ADR-0010); aozora-md-side follow-up is usually a one-line mapping in
      `aozora_flavored_markdown::ir` plus a test.
- [ ] If this changes the renderer-emitted class set: styled it in both
      `crates/aozora-flavored-markdown/theme/` files (`aozora-md-horizontal.css`
      / `aozora-md-vertical.css`). `classes::all()` derives itself.

## Related

<!-- Closes #N / part of #M / cross-reference to ADR-NNNN, etc. -->
