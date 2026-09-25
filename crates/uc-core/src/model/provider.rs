//! Port of upstream `Provider.swift`.

use serde::{Deserialize, Serialize};

/// One external quick-link button on a provider card.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProviderLink {
    pub label: String,
    pub url: String,
}

impl ProviderLink {
    pub fn new(label: impl Into<String>, url: impl Into<String>) -> Self {
        Self { label: label.into(), url: url.into() }
    }
}

/// A data source that can register widgets it knows how to feed.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Provider {
    /// Stable identifier, e.g. `claude`, or `claude@1a2b3c4d` for an extra account card.
    pub id: String,
    pub display_name: String,
    /// The provider family whose mark and brand color this card uses (`claude` for `claude@…`).
    pub icon: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<ProviderLink>,
}

impl Provider {
    pub fn new(id: impl Into<String>, display_name: impl Into<String>) -> Self {
        let id = id.into();
        let icon = Self::family_of(&id).to_string();
        Self { id, display_name: display_name.into(), icon, links: Vec::new() }
    }

    pub fn with_links(mut self, links: Vec<ProviderLink>) -> Self {
        self.links = links;
        self
    }

    /// The provider family an account-scoped id belongs to (`claude@1a2b3c4d` → `claude`).
    pub fn family_of(id: &str) -> &str {
        id.split_once('@').map_or(id, |(family, _)| family)
    }

    pub fn family(&self) -> &str {
        Self::family_of(&self.id)
    }

    /// Links safe to render: trimmed, non-empty label and URL, `http(s)` scheme only.
    pub fn visible_links(&self) -> Vec<ProviderLink> {
        self.links
            .iter()
            .filter_map(|link| {
                let label = link.label.trim();
                let url = link.url.trim();
                let allowed = url.starts_with("https://") || url.starts_with("http://");
                (!label.is_empty() && !url.is_empty() && allowed).then(|| ProviderLink::new(label, url))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn account_ids_resolve_to_their_family_icon() {
        let provider = Provider::new("claude@1a2b3c4d", "Claude");
        assert_eq!(provider.icon, "claude");
        assert_eq!(provider.family(), "claude");
    }

    #[test]
    fn visible_links_drop_unsafe_entries() {
        let provider = Provider::new("codex", "Codex").with_links(vec![
            ProviderLink::new(" Status ", " https://status.openai.com "),
            ProviderLink::new("", "https://example.com"),
            ProviderLink::new("Local", "file:///etc/passwd"),
        ]);
        assert_eq!(provider.visible_links(), vec![ProviderLink::new("Status", "https://status.openai.com")]);
    }
}
