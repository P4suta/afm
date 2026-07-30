# fuzz regressions

Permanent regression cases for the cargo-fuzz harnesses under
`crates/aozora-flavored-markdown/fuzz/`. The integration test
`tests/fuzz_regressions.rs` walks every input file in this tree on
each `just test` run, so a once-fixed crash stays fixed even if
nightly rustc / libFuzzer drift away from the original repro.

## Layout

```
tests/fuzz_regressions/
  parse_render/
    crash-<sha>            # raw byte payload, fed verbatim
    crash-<sha>.expect.txt # (optional) panic snippet, archaeology only
  render_blocks/
    .gitkeep               # nothing promoted yet — the directory still exists
  canonicalize_round_trip/
    ...
  sjis_decode/
    ...
```

The runner picks files up by directory walk — drop a new file in and
`just test` will replay it without further plumbing.

Every registered fuzz target owns a directory here whether or not it has
anything pinned, and `.gitkeep` is what keeps an empty one in git. A
missing directory is a hard failure, not an empty replay: `read_dir`
answered "no such directory" with an empty list, an empty list is a pass,
and `render_blocks`, `canonicalize_round_trip` and `sjis_decode` therefore
reported green over a directory that had never been created.

## Workflow

1. Run a fuzz harness:

   ```sh
   just fuzz-quick parse_render          # 60 s
   just fuzz-deep  parse_render          # 5 min — release gate
   just fuzz-all-quick                   # every target, 60 s each
   just fuzz-all-deep                    # every target, 5 min each
   ```

2. If libFuzzer flags a crash, it writes the offending input to
   `crates/aozora-flavored-markdown/fuzz/artifacts/<target>/crash-<sha>` and
   exits non-zero.

3. Triage the unknown panic — every artifact, one shell call,
   panic-line filtered out of the libFuzzer noise:

   ```sh
   just fuzz-triage parse_render
   ```

   Each artifact prints either the `panicked at … Tier X violated`
   line or, if the harness exited cleanly, the trailing 5 lines of
   its output. Exit status is the count of crashing artifacts so
   the recipe can drive a CI gate.

4. Fix the underlying issue, then promote the artifact so future
   `just test` runs pin the fix:

   ```sh
   just fuzz-promote parse_render crash-<sha>
   ```

   This moves the artifact into `tests/fuzz_regressions/<target>/`,
   where `tests/fuzz_regressions.rs` picks it up on every run.

5. Sanity-check the regression set is what you expect:

   ```sh
   just fuzz-status
   # target              pending_crashes  pinned_regressions
   # --------------------------------------------------------
   # parse_render        0                3
   # render_blocks       0                0
   # canonicalize_round_trip 0             1
   # sjis_decode         0                0
   ```

`pending_crashes > 0` is the call-to-action: triage and either fix
or promote.

## Diagnostic philosophy

Every harness funnels its assertions through
`aozora_flavored_markdown_test_support::assert_html_invariants(src, &html)`, which
panics with a self-contained message:

```
Tier I (double-encoded entity) violated:
  src = "\\&amp;\r\u{e010}\\"
  html = "<p>&amp;amp;<br />\n\u{e010}\\</p>\n"
  details = DoubleEncodedEntity { snippet: "..." }
```

You should never need a stack-trace to know what failed — `panicked
at` line + the next two lines are enough to reproduce the failure
anywhere.
