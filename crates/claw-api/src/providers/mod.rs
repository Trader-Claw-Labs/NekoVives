pub mod anthropic;
pub mod openai_compat;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderKind {
    Anthropic,
    OpenAiCompat,
}

/// Model alias table — short names expand to full model IDs.
const MODEL_ALIASES: &[(&str, &str)] = &[
    ("opus", "claude-opus-4-6"),
    ("sonnet", "claude-sonnet-4-6"),
    ("haiku", "claude-haiku-4-5-20251001"),
    ("grok", "grok-3"),
    ("grok-mini", "grok-3-mini"),
];

/// Expand a short alias to a full model ID, or return as-is.
#[must_use]
pub fn resolve_model_alias(model: &str) -> String {
    MODEL_ALIASES
        .iter()
        .find(|(alias, _)| *alias == model)
        .map(|(_, full)| full.to_string())
        .unwrap_or_else(|| model.to_string())
}

/// Infer the provider kind from a model name.
#[must_use]
pub fn detect_provider_kind(model: &str) -> ProviderKind {
    let lower = model.to_ascii_lowercase();
    if lower.starts_with("claude") {
        ProviderKind::Anthropic
    } else {
        ProviderKind::OpenAiCompat
    }
}
