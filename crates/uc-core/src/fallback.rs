//! Display helpers for fallback-priced models. Port of the string half of upstream
//! `PricingFallbackOptions.swift` (the rate lookup lives in `uc-pricing`).

use std::collections::BTreeSet;

use crate::model::ModelsByDay;

/// Human title for a public model id: `gpt-5.4-mini` → `GPT 5.4 Mini`.
pub fn fallback_title(model: &str) -> String {
    model
        .split('-')
        .map(|part| match part {
            "gpt" => "GPT".to_string(),
            "o1" | "o3" | "o4" => part.to_string(),
            _ => {
                let mut chars = part.chars();
                match chars.next() {
                    Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                    None => String::new(),
                }
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

/// Append " · Fallback estimates: …" when any of `days` used fallback pricing.
pub fn source_note_with_fallbacks(note: &str, models_by_day: Option<&ModelsByDay>, days: &BTreeSet<String>) -> String {
    let models: BTreeSet<&String> = days
        .iter()
        .filter_map(|day| models_by_day.and_then(|map| map.get(day)))
        .flatten()
        .collect();
    if models.is_empty() {
        return note.to_string();
    }
    let names: Vec<String> = models.into_iter().map(|model| fallback_title(model)).collect();
    format!("{note} · Fallback estimates: {}", names.join(", "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_follow_upstream_casing() {
        assert_eq!(fallback_title("gpt-5.4-mini"), "GPT 5.4 Mini");
        assert_eq!(fallback_title("o3-pro"), "o3 Pro");
        assert_eq!(fallback_title("claude-sonnet-4-5"), "Claude Sonnet 4 5");
    }

    #[test]
    fn notes_list_fallback_models_for_the_period() {
        let mut by_day = ModelsByDay::new();
        by_day.insert("2026-09-25".into(), BTreeSet::from(["gpt-5.4".to_string()]));
        let days = BTreeSet::from(["2026-09-25".to_string()]);
        assert_eq!(
            source_note_with_fallbacks("From your Codex usage history (estimated)", Some(&by_day), &days),
            "From your Codex usage history (estimated) · Fallback estimates: GPT 5.4"
        );
        let other = BTreeSet::from(["2026-09-24".to_string()]);
        assert_eq!(source_note_with_fallbacks("Note", Some(&by_day), &other), "Note");
    }
}
