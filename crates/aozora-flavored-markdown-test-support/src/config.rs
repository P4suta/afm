//! Shared proptest configuration.
//!
//! One environment variable (`AOZORA_PROPTEST_CASES`) tunes the whole sweep.
//! The 128-case default balances catching regressions during `just test`
//! against keeping each proptest binary under a few seconds in CI.

use std::env;

use proptest::prelude::ProptestConfig;
use proptest::test_runner::FileFailurePersistence;

const CASES_ENV: &str = "AOZORA_PROPTEST_CASES";
const DEFAULT_CASES: u32 = 128;

/// Persisted failure seeds land in each test's `proptest-regressions/` file,
/// which is committed, so a past failure replays first on every later run.
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

/// Unset, unparsable *or zero* falls back to [`DEFAULT_CASES`]. Zero matters
/// on its own: proptest accepts `cases: 0`, replays the persisted seeds,
/// generates nothing and reports success — so a stray `=0` would delete the
/// sweep while leaving CI green.
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

    #[test]
    fn a_zero_case_count_falls_back_instead_of_skipping_the_sweep() {
        assert_eq!(cases_from(Some("0")), DEFAULT_CASES);
        assert_eq!(cases_from(Some("00")), DEFAULT_CASES);
    }
}
