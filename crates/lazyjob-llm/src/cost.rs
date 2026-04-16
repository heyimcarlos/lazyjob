// Microdollar pricing table per 1000 tokens (blended input/output rates).
// 1 microdollar = $0.000001. Rates are approximate public list prices.
const PRICING: &[(&str, u64)] = &[
    ("claude-opus", 45_000),
    ("claude-sonnet", 9_000),
    ("claude-haiku", 750),
    ("gpt-4o-mini", 375),
    ("gpt-4o", 10_000),
    ("gpt-4", 45_000),
    ("gpt-3.5", 1_000),
    ("llama", 0),
    ("mistral", 0),
    ("nomic", 0),
];

const DEFAULT_RATE: u64 = 5_000;

pub fn estimate_cost(model: &str, tokens: u32) -> u64 {
    let rate = PRICING
        .iter()
        .find(|(prefix, _)| model.contains(prefix))
        .map(|(_, rate)| *rate)
        .unwrap_or(DEFAULT_RATE);
    (tokens as u64).saturating_mul(rate) / 1_000
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_zero_tokens() {
        assert_eq!(estimate_cost("claude-haiku-4-5", 0), 0);
    }

    #[test]
    fn estimate_cost_claude_haiku() {
        // 1000 tokens at 750 microdollars/1k = 750 microdollars = $0.00075
        assert_eq!(estimate_cost("claude-haiku-4-5-20251001", 1_000), 750);
    }

    #[test]
    fn estimate_cost_gpt4o_mini() {
        // 1000 tokens at 375 microdollars/1k = 375 microdollars
        assert_eq!(estimate_cost("gpt-4o-mini", 1_000), 375);
    }

    #[test]
    fn estimate_cost_ollama_is_free() {
        assert_eq!(estimate_cost("llama3.2:latest", 100_000), 0);
    }

    #[test]
    fn estimate_cost_unknown_model_uses_default() {
        // Default rate: 5000 microdollars/1k tokens
        assert_eq!(estimate_cost("some-unknown-model-v99", 1_000), 5_000);
    }

    #[test]
    fn estimate_cost_claude_opus() {
        // 1M tokens at 45k microdollars/1k = 45_000_000 microdollars = $45
        assert_eq!(estimate_cost("claude-opus-4", 1_000_000), 45_000_000);
    }

    #[test]
    fn estimate_cost_gpt4o() {
        // 10k tokens at 10_000 microdollars/1k = 100_000 microdollars = $0.10
        assert_eq!(estimate_cost("gpt-4o", 10_000), 100_000);
    }
}
