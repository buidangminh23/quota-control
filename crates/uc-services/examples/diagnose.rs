//! Lists the logins and environment keys every service finds on this computer and what one refresh
//! of each reads, without printing any credential: `cargo run -p uc-services --example diagnose`.
//! Pass service ids to limit it (`-- gemini ollama`).

use uc_core::{MetricLine, ReqwestHttpClient};

fn mask(label: &str) -> String {
    match label.split_once('@') {
        Some((name, domain)) => {
            let keep: String = name.chars().take(2).collect();
            format!("{keep}***@{domain}")
        }
        None => label.to_string(),
    }
}

fn describe(line: &MetricLine) -> String {
    match line {
        MetricLine::Progress(line) => format!(
            "{}: {:.1} / {:.1}{}",
            line.label,
            line.used,
            line.limit,
            line.resets_at
                .map(|time| format!(" (resets {time})"))
                .unwrap_or_default()
        ),
        MetricLine::Values(line) => format!(
            "{}: {}",
            line.label,
            line.values
                .iter()
                .map(|value| format!("{:.2} {:?}", value.number, value.kind))
                .collect::<Vec<_>>()
                .join(", ")
        ),
        MetricLine::Badge(line) => format!("{}: [{}]", line.label, line.text),
        MetricLine::Text(line) => format!("{}: {}", line.label, line.value),
        MetricLine::Chart(line) => format!("{}: {} points", line.label, line.points.len()),
    }
}

#[tokio::main]
async fn main() {
    let only: Vec<String> = std::env::args().skip(1).collect();
    let roots = uc_services::Roots::system();
    let store = uc_accounts::KeyStore::default_store();
    let saved = store.list().unwrap_or_default();
    let detected: Vec<_> = uc_services::detect(&roots, &saved)
        .into_iter()
        .filter(|found| only.is_empty() || only.iter().any(|id| id == found.service))
        .collect();
    println!("services compiled in: {}", uc_services::services().len());
    println!("cards found: {}", detected.len());
    let cards = uc_services::runtimes(&detected, &store, &[], &roots, ReqwestHttpClient::shared());
    for (found, card) in detected.iter().zip(cards) {
        println!(
            "\n{} · {} (from {})",
            found.service,
            found
                .label
                .as_deref()
                .map(mask)
                .unwrap_or_else(|| "-".into()),
            found.origin
        );
        let snapshot = card.refresh(uc_core::RefreshContext::manual()).await;
        if let Some(category) = snapshot.error_category {
            println!(
                "  error {}: {}",
                category.as_str(),
                snapshot.error_text().unwrap_or_default()
            );
            continue;
        }
        println!("  plan: {}", snapshot.plan.as_deref().unwrap_or("-"));
        for line in &snapshot.lines {
            println!("  {}", describe(line));
        }
    }
}
