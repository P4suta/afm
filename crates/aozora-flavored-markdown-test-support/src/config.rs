//! Shared proptest configuration.
//!
//! Every property test in the workspace draws its [`ProptestConfig`] from
//! [`default_config`] so one environment variable tunes the whole sweep. The
//! default of 128 cases is the compromise between catching regressions during
//! `just test` and keeping each proptest binary under a few seconds in CI.
//!
//! Override sites:
//!
//! * `AOZORA_PROPTEST_CASES=16` for tight local iteration.
//! * `AOZORA_PROPTEST_CASES=4096` for the pre-release deep sweep
//!   (`just prop-deep`).

use std::env;

use proptest::prelude::ProptestConfig;
use proptest::test_runner::FileFailurePersistence;

/// Environment variable that overrides the per-property case count.
const CASES_ENV: &str = "AOZORA_PROPTEST_CASES";

/// Number of cases each property runs when `AOZORA_PROPTEST_CASES` is unset.
const DEFAULT_CASES: u32 = 128;

/// The [`ProptestConfig`] every property test in this workspace uses.
///
/// * `cases` reads `AOZORA_PROPTEST_CASES`, falling back to 128.
/// * `max_shrink_iters` stays at proptest's own 10 000 so shrinking converges
///   on a minimal counterexample without blowing the per-run budget.
/// * `failure_persistence` writes seeds into each test's
///   `proptest-regressions/` file — those are committed, so a past failure
///   replays first on every subsequent run.
#[must_use]
pub fn default_config() -> ProptestConfig {
    ProptestConfig {
        cases: cases_from(env::var(CASES_ENV).ok().as_deref()),
        max_shrink_iters: 10_000,
        failure_persistence: Some(Box::new(FileFailurePersistence::WithSource(
            "proptest-regressions",
        ))),
        ..ProptestConfig::default()
    }
}

/// Resolve the case count from a raw environment value.
///
/// An unset, unparsable *or zero* value yields [`DEFAULT_CASES`] rather than
/// a panic: a misconfigured CI job must not be able to swap strict testing
/// for a crash that reads as an infrastructure fault, nor for a silent skip.
///
/// Zero is the one value that would produce exactly that silent skip.
/// `cases: 0` is not an error to proptest — the runner replays the persisted
/// regression seeds, generates nothing, and reports success, so the sweep
/// disappears while the job stays green.
fn cases_from(raw: Option<&str>) -> u32 {
    raw.and_then(|s| s.parse().ok())
        .filter(|&cases| cases > 0)
        .unwrap_or(DEFAULT_CASES)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unset_case_count_falls_back_to_the_default() {
        assert_eq!(cases_from(None), DEFAULT_CASES);
    }

    #[test]
    fn a_parseable_case_count_is_honoured() {
        assert_eq!(cases_from(Some("4096")), 4096);
    }

    #[test]
    fn an_unparsable_case_count_falls_back_instead_of_panicking() {
        for raw in ["", "  ", "lots", "-1", "1.5", "99999999999999999999"] {
            assert_eq!(
                cases_from(Some(raw)),
                DEFAULT_CASES,
                "{raw:?} must fall back, not panic or silently weaken the sweep"
            );
        }
    }

    /// The one value proptest *would* accept while generating nothing:
    /// `cases: 0` replays the persisted seeds and reports success, so a
    /// stray `AOZORA_PROPTEST_CASES=0` would leave CI green with the sweep
    /// switched off.
    #[test]
    fn a_zero_case_count_falls_back_instead_of_skipping_the_sweep() {
        assert_eq!(cases_from(Some("0")), DEFAULT_CASES);
        assert_eq!(cases_from(Some("00")), DEFAULT_CASES);
    }
}
