# Bundled estimated pricing

`data/rates.json` contains 144 exact Claude/OpenAI model entries selected from
OpenUsage's `Sources/OpenUsage/Resources/pricing_litellm_snapshot.json` as present
in this repository's upstream reference. The upstream snapshot retrieval date
is **2026-07-02T12:43:06Z**. Its MIT license is retained in `LICENSE-UPSTREAM`.

The fields and units are preserved from upstream: `i`, `o`, `cr`, `cw` are USD
per million input, output, cache-read and five-minute cache-write tokens. Their
`a` variants apply above 200,000 prompt tokens; `fast` is an explicit multiplier.
One-hour Claude cache creation uses twice the applicable input rate, matching
the upstream ModelRates implementation.

Pricing only accepts exact names. Unknown/internal/new models, unsupported fast
rates, invalid token counts, and aggregate usage whose request boundaries are
unknown for a tiered model return `None`. There is no name guessing or fallback
to an unrelated model. Cached Codex input must first be removed from ordinary
input before calling `estimate`; scanner normalization handles this.

These are fixed bundled **API-equivalent estimates**, not current prices,
historical invoices, quota consumption, or subscription charges. They are not
automatically refreshed from the network.
