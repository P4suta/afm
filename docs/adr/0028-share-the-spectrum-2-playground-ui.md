# 0028. Share the Spectrum 2 playground UI from the Aozora repository

- Status: accepted
- Date: 2026-07-30
- Deciders: @P4suta
- Tags: playground, web, spectrum, accessibility, vendoring

## Context

The Aozora parser and Aozora Flavored Markdown renderer need different URLs,
WASM entry points, examples, notation help, and rendered-document CSS. They do
not need two unrelated application shells. The old playgrounds duplicated
Solid components, browser persistence, share URLs, responsive behavior, and
accessibility decisions. They also exposed parser and serializer data that is
useful for engine debugging but distracts from the public playground's primary
job: helping an author write and preview a document.

The repositories cannot consume a floating branch without making either site
non-reproducible. Publishing the UI to npm would add a release stream for code
that has exactly two first-party consumers.

## Decision

`P4suta/aozora` owns a private `playground-ui/` React 19 package. This
repository carries an allowlisted byte-for-byte snapshot in
`playground/vendor/playground-ui/`. The package renders the application frame,
state transitions, persistence and migrations, internationalized catalog,
explicit share operation, responsive panes and dialogs, diagnostics, outline,
and command palette. It only talks to a product through
`PlaygroundAdapter`; renderer HTML remains behind the product-owned preview
controller's `innerHTML` boundary.

Adobe Spectrum 2 is the only UI design system. The package uses React Spectrum
components, Spectrum icons, Provider, and the official style macro. It does
not use `UNSAFE_*` styling, product colors, emoji controls, or selectors that
depend on Spectrum's internal classes. Local CSS is restricted to CodeMirror,
renderer-owned document CSS, and pane sizing.

Shared preferences use one cross-site key; drafts and editor settings use
product-scoped keys. A share URL is created only when the author invokes
Share. New hashes use `#src=<lz-string>` while the old AFM hash and Aozora
`?text` and `?c` forms remain readable. On startup an explicit share takes
precedence over a product draft, which takes precedence over the first sample.

WASM and CodeMirror load behind a dynamic adapter boundary so Spectrum can
paint the application frame first. Spectrum's Provider is retained, but its
Typekit network loader is replaced at build time: GitHub Pages cannot add
font response headers, the production CSP is same-origin only, and Spectrum's
own font stacks include system fallbacks. No UI tokens or component styling
are changed.

## Vendoring and release sequence

The update flow is deliberately two pull requests:

1. change and verify `playground-ui/` in `aozora`, then merge it;
2. run `bun run vendor:sync -- /path/to/aozora/playground-ui` here, verify the
   generated lock, and merge the AFM adapter update.

The sync command rejects uncommitted canonical input and copies only the
allowlist in `scripts/playground-ui-files.ts`, removing stale snapshot files.
The lock records the upstream commit, repository tree, package tree, allowlist,
and a framed SHA-256 digest. `vendor:verify` requires the exact allowlisted file
set, checks local integrity, and, when given a checkout at the locked commit,
compares every allowed file byte for byte.

The committed consumer snapshot is always `locked` to an existing canonical
package tree. A temporary `bootstrap` lock may exist only between the two
ordered repository changes; it cannot pass comparison against an upstream
checkout and is replaced by the first canonical sync.

## Verification

The shared package is exercised with a fake adapter for initialization,
failure and retry, stale async results, commands, diagnostics, outline,
lifecycle, persistence, URL compatibility, and localization. A reusable
contract suite is applied to the real AFM adapter and ships with the vendor
package for the Aozora adapter.

Production-browser tests cover desktop and 320 px mobile layouts, real WASM
rendering, keyboard and focus behavior, legacy boot state, page scrolling,
CSP and same-origin resources, WCAG 2.2 AA with axe, and screenshot regression.
Lighthouse CI and compressed bundle budgets guard performance, accessibility,
best practices, SEO, JavaScript, CSS, and WASM transfer regressions.

## Consequences

The two playgrounds retain their engines and public URLs but converge on one
author-facing interaction model. UI changes wait for an Aozora merge before
the consumer snapshot moves; that coordination cost is intentional and
visible in the lock.

Debug JSON and serialization views are no longer public-playground features.
They remain available to API, CLI, and engine tests. A future inspector is a
separate product decision rather than a reason to grow developer output back
into the authoring shell.
