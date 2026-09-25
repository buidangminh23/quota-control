use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;

pub const PRICING_SOURCE: &str = "Bundled OpenUsage snapshot (2026-07-02) with official model updates verified 2026-09-26; API-equivalent estimate, not subscription charges";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    pub input: i64,
    pub output: i64,
    pub cache_read: i64,
    pub cache_write: i64,
    pub cache_write_hour: i64,
    pub fast: bool,
}

impl Tokens {
    pub fn prompt(self) -> i64 {
        self.input
            .saturating_add(self.cache_read)
            .saturating_add(self.cache_write)
            .saturating_add(self.cache_write_hour)
    }

    pub fn total(self) -> i64 {
        self.prompt().saturating_add(self.output)
    }
}

#[derive(Debug, Deserialize)]
struct Rates {
    i: f64,
    o: f64,
    cr: f64,
    cw: f64,
    ia: Option<f64>,
    oa: Option<f64>,
    cra: Option<f64>,
    cwa: Option<f64>,
    cwh: Option<f64>,
    long_context_threshold: Option<i64>,
    #[serde(default)]
    hour_cache_unsupported: bool,
    fast: Option<f64>,
}

static RATES: LazyLock<BTreeMap<String, Rates>> = LazyLock::new(|| {
    let mut rates: BTreeMap<String, Rates> =
        serde_json::from_str(include_str!("../data/rates.json"))
            .expect("validated bundled pricing");
    let verified: BTreeMap<String, Rates> =
        serde_json::from_str(include_str!("../data/verified-rates.json"))
            .expect("validated official model pricing");
    rates.extend(verified);
    rates
});

pub fn estimate(model: &str, tokens: Tokens, request_boundaries_known: bool) -> Option<f64> {
    let rates = RATES.get(model)?;
    if [
        tokens.input,
        tokens.output,
        tokens.cache_read,
        tokens.cache_write,
        tokens.cache_write_hour,
    ]
    .iter()
    .any(|v| *v < 0)
    {
        return None;
    }
    if !request_boundaries_known
        && [rates.ia, rates.oa, rates.cra, rates.cwa]
            .iter()
            .any(Option::is_some)
    {
        return None;
    }
    if tokens.cache_write_hour > 0 && rates.hour_cache_unsupported {
        return None;
    }
    let high = tokens.prompt() > rates.long_context_threshold.unwrap_or(200_000);
    let choose = |base, higher: Option<f64>| if high { higher.unwrap_or(base) } else { base };
    let input = choose(rates.i, rates.ia);
    let multiplier = if tokens.fast { rates.fast? } else { 1.0 };
    let cost = (tokens.input as f64 * input
        + tokens.output as f64 * choose(rates.o, rates.oa)
        + tokens.cache_read as f64 * choose(rates.cr, rates.cra)
        + tokens.cache_write as f64 * choose(rates.cw, rates.cwa)
        + tokens.cache_write_hour as f64 * rates.cwh.unwrap_or(input * 2.0))
        / 1_000_000.0
        * multiplier;
    cost.is_finite().then_some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_cost(model: &str, tokens: Tokens, expected: f64) {
        let actual = estimate(model, tokens, true).expect(model);
        assert!(
            (actual - expected).abs() < 1e-10,
            "{model}: expected {expected}, got {actual}"
        );
    }

    #[test]
    fn verified_models_charge_each_token_bucket_at_its_official_rate() {
        for (model, input, output, read, write, hour) in [
            ("claude-opus-5-5", 4.0, 20.0, 0.2, 5.0, Some(8.0)),
            ("claude-opus-5", 5.0, 25.0, 0.5, 6.25, Some(10.0)),
            ("claude-fable-5-1", 10.0, 50.0, 0.25, 12.5, Some(20.0)),
            ("gpt-6-astra", 10.0, 50.0, 1.0, 12.5, None),
            ("gpt-5.6-luna", 0.2, 1.2, 0.02, 0.25, None),
            ("gpt-6-luna", 0.1, 0.5, 0.01, 0.125, None),
        ] {
            for (tokens, rate) in [
                (
                    Tokens {
                        input: 1_000,
                        ..Tokens::default()
                    },
                    input,
                ),
                (
                    Tokens {
                        output: 1_000,
                        ..Tokens::default()
                    },
                    output,
                ),
                (
                    Tokens {
                        cache_read: 1_000,
                        ..Tokens::default()
                    },
                    read,
                ),
                (
                    Tokens {
                        cache_write: 1_000,
                        ..Tokens::default()
                    },
                    write,
                ),
            ] {
                assert_cost(model, tokens, rate / 1_000.0);
            }
            let hourly = Tokens {
                cache_write_hour: 1_000,
                ..Tokens::default()
            };
            if let Some(rate) = hour {
                assert_cost(model, hourly, rate / 1_000.0);
            } else {
                assert!(estimate(model, hourly, true).is_none());
            }
        }
    }

    #[test]
    fn openai_long_context_threshold_counts_all_prompt_buckets_and_reprices_full_request() {
        for (model, input, output, read, write) in [
            ("gpt-6-astra", 10.0, 50.0, 1.0, 12.5),
            ("gpt-5.6-luna", 0.2, 1.2, 0.02, 0.25),
            ("gpt-6-luna", 0.1, 0.5, 0.01, 0.125),
        ] {
            for prompt in [200_001, 271_999, 272_000, 272_001] {
                let tokens = Tokens {
                    input: prompt - 200_000,
                    cache_read: 100_000,
                    cache_write: 100_000,
                    output: 1_000,
                    ..Tokens::default()
                };
                let (input_multiplier, output_multiplier) = if prompt > 272_000 {
                    (2.0, 1.5)
                } else {
                    (1.0, 1.0)
                };
                let expected = ((tokens.input as f64 * input + 100_000.0 * (read + write))
                    * input_multiplier
                    + 1_000.0 * output * output_multiplier)
                    / 1_000_000.0;
                assert_cost(model, tokens, expected);
                assert_cost(
                    model,
                    Tokens {
                        fast: true,
                        ..tokens
                    },
                    expected * 2.0,
                );
                assert!(estimate(model, tokens, false).is_none());
            }
            assert!(estimate(
                model,
                Tokens {
                    input: 1,
                    ..Tokens::default()
                },
                false
            )
            .is_none());
        }
    }

    #[test]
    fn verified_claude_has_no_long_context_surcharge_and_only_documented_fast_modes() {
        for (model, input, read, write, hour, supports_fast) in [
            ("claude-opus-5-5", 4.0, 0.2, 5.0, 8.0, true),
            ("claude-opus-5", 5.0, 0.5, 6.25, 10.0, true),
            ("claude-fable-5-1", 10.0, 0.25, 12.5, 20.0, false),
        ] {
            let tokens = Tokens {
                input: 600_000,
                cache_read: 100_000,
                cache_write: 100_000,
                cache_write_hour: 100_000,
                ..Tokens::default()
            };
            let expected = 0.6 * input + 0.1 * (read + write + hour);
            assert_cost(model, tokens, expected);
            assert_eq!(
                estimate(model, tokens, false),
                estimate(model, tokens, true)
            );
            let fast = Tokens {
                fast: true,
                ..tokens
            };
            if supports_fast {
                assert_cost(model, fast, expected * 2.0);
            } else {
                assert!(estimate(model, fast, true).is_none());
            }
        }
    }

    #[test]
    fn legacy_threshold_and_hourly_cache_behavior_are_preserved() {
        for prompt in [200_000, 200_001] {
            let tokens = Tokens {
                input: prompt - 100_000,
                cache_write_hour: 100_000,
                ..Tokens::default()
            };
            let input_rate = if prompt > 200_000 { 6.0 } else { 3.0 };
            assert_cost(
                "claude-sonnet-4-20250514",
                tokens,
                (tokens.input as f64 + 200_000.0) * input_rate / 1_000_000.0,
            );
        }
    }

    #[test]
    fn verified_entries_include_dated_primary_source_provenance() {
        let verified: serde_json::Value =
            serde_json::from_str(include_str!("../data/verified-rates.json")).unwrap();
        assert_eq!(verified.as_object().unwrap().len(), 6);
        for (model, entry) in verified.as_object().unwrap() {
            assert_eq!(entry["verified_on"], "2026-09-26");
            let sources = entry["sources"].as_array().unwrap();
            assert!(sources.len() >= 2, "{model}");
            assert!(sources.iter().all(|source| {
                let url = source.as_str().unwrap();
                url.starts_with("https://platform.claude.com/docs/")
                    || url.starts_with("https://developers.openai.com/api/docs/")
            }));
            assert!(RATES.contains_key(model));
        }
    }

    #[test]
    fn exact_models_only_and_no_invented_fast_rates() {
        assert!(estimate("gpt-future", Tokens::default(), true).is_none());
        assert!(estimate(
            "gpt-5",
            Tokens {
                fast: true,
                ..Tokens::default()
            },
            true
        )
        .is_none());
        assert!(estimate("claude-sonnet-4-20250514", Tokens::default(), false).is_none());
    }

    #[test]
    fn cached_input_is_not_billed_twice() {
        let cost = estimate(
            "gpt-5",
            Tokens {
                input: 800,
                cache_read: 200,
                output: 100,
                ..Tokens::default()
            },
            true,
        )
        .unwrap();
        assert!((cost - 0.002025).abs() < 1e-10);
    }
}
