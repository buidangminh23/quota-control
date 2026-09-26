const MILLION: i64 = 1_000_000;
const STANDARD: i64 = 200_000;
const _VERIFIED_SOURCE: &str =
    "2026-09-26 https://platform.claude.com/docs/en/build-with-claude/context-windows";

const MILLION_MODELS: &[&str] = &[
    "claude-fable-5-1",
    "claude-mythos-5-1",
    "claude-fable-5",
    "claude-mythos-5",
    "claude-mythos-preview",
    "claude-opus-5-5",
    "claude-opus-5",
    "claude-opus-4-8",
    "claude-opus-4-7",
    "claude-opus-4-6",
    "claude-sonnet-5",
    "claude-sonnet-4-6",
];

const STANDARD_MODELS: &[&str] = &[
    "claude-haiku-4-5",
    "claude-sonnet-4-5",
    "claude-sonnet-4",
    "claude-4-sonnet",
    "claude-opus-4-5",
    "claude-opus-4-1",
    "claude-opus-4",
    "claude-4-opus",
    "claude-3-7-sonnet",
    "claude-3-5-sonnet",
    "claude-3-5-haiku",
    "claude-3-sonnet",
    "claude-3-haiku",
    "claude-3-opus",
];

pub(crate) fn claude_window(
    model: &str,
    evidence: i64,
    configured_model: Option<&str>,
) -> Option<i64> {
    let plain = model.strip_suffix("[1m]").unwrap_or(model);
    if MILLION_MODELS
        .iter()
        .any(|known| matches_model(plain, known))
    {
        return Some(MILLION);
    }
    if !STANDARD_MODELS
        .iter()
        .any(|known| matches_model(plain, known))
    {
        return None;
    }
    let configured_million = configured_model
        .and_then(|configured| configured.strip_suffix("[1m]"))
        .is_some_and(|configured| {
            matches_model(plain, configured)
                || matches!(configured, "opus" | "sonnet" | "haiku")
                    && plain.starts_with(&format!("claude-{configured}-"))
        });
    if model.ends_with("[1m]") || configured_million || (STANDARD < evidence && evidence <= MILLION)
    {
        Some(MILLION)
    } else if evidence > MILLION {
        None
    } else {
        Some(STANDARD)
    }
}

fn matches_model(model: &str, known: &str) -> bool {
    model == known
        || model.strip_prefix(known).is_some_and(|suffix| {
            suffix.strip_prefix('-').is_some_and(|date| {
                date.len() == 8 && date.bytes().all(|byte| byte.is_ascii_digit())
            })
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn official_model_windows_are_explicit_and_unknown_models_stay_unknown() {
        for model in MILLION_MODELS {
            assert_eq!(claude_window(model, 1000, None), Some(MILLION));
        }
        for model in STANDARD_MODELS {
            assert_eq!(claude_window(model, 1000, None), Some(STANDARD));
        }
        assert_eq!(
            claude_window("claude-haiku-4-5-20251001", 1000, None),
            Some(STANDARD)
        );
        for unknown in [
            "Unattributed",
            "claude-opus-99",
            "claude-opus-5-unknown",
            "gpt-6-sol",
        ] {
            assert_eq!(claude_window(unknown, 500_000, Some("opus[1m]")), None);
        }
    }

    #[test]
    fn extended_mode_uses_session_evidence_or_matching_configuration() {
        assert_eq!(
            claude_window("claude-sonnet-4-5", 200_000, None),
            Some(STANDARD)
        );
        assert_eq!(
            claude_window("claude-sonnet-4-5", 200_001, None),
            Some(MILLION)
        );
        assert_eq!(claude_window("claude-sonnet-4-5", 1_000_001, None), None);
        assert_eq!(
            claude_window("claude-sonnet-4-5", 1000, Some("sonnet[1m]")),
            Some(MILLION)
        );
        assert_eq!(
            claude_window("claude-opus-4-5", 1000, Some("opus[1m]")),
            Some(MILLION)
        );
        assert_eq!(
            claude_window("claude-sonnet-4-5", 1000, Some("opus[1m]")),
            Some(STANDARD)
        );
        assert_eq!(
            claude_window("claude-sonnet-4-5[1m]", 1000, None),
            Some(MILLION)
        );
    }
}
