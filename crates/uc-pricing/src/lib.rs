use std::collections::BTreeMap;
use std::sync::LazyLock;

use serde::Deserialize;

pub const PRICING_SOURCE: &str = "Bundled OpenUsage LiteLLM snapshot (2026-07-02); API-equivalent estimate, not subscription charges";

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
    fast: Option<f64>,
}

static RATES: LazyLock<BTreeMap<String, Rates>> = LazyLock::new(|| {
    serde_json::from_str(include_str!("../data/rates.json")).expect("validated bundled pricing")
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
    if !request_boundaries_known && rates.ia.is_some() {
        return None;
    }
    let high = tokens.prompt() > 200_000;
    let choose = |base, higher: Option<f64>| if high { higher.unwrap_or(base) } else { base };
    let input = choose(rates.i, rates.ia);
    let multiplier = if tokens.fast { rates.fast? } else { 1.0 };
    let cost = (tokens.input as f64 * input
        + tokens.output as f64 * choose(rates.o, rates.oa)
        + tokens.cache_read as f64 * choose(rates.cr, rates.cra)
        + tokens.cache_write as f64 * choose(rates.cw, rates.cwa)
        + tokens.cache_write_hour as f64 * input * 2.0)
        / 1_000_000.0
        * multiplier;
    cost.is_finite().then_some(cost)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_models_only_and_no_invented_fast_rates() {
        assert!(estimate("gpt-future", Tokens::default(), true).is_none());
        assert!(
            estimate(
                "gpt-5",
                Tokens {
                    fast: true,
                    ..Tokens::default()
                },
                true
            )
            .is_none()
        );
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
