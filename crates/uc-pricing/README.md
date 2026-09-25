# Bundled estimated pricing

`data/rates.json` contains 144 exact Claude/OpenAI model entries selected from
OpenUsage's `Sources/OpenUsage/Resources/pricing_litellm_snapshot.json` as present
in this repository's upstream reference. The upstream snapshot retrieval date
is **2026-07-02T12:43:06Z**. Its MIT license is retained in `LICENSE-UPSTREAM`.

The fields and units are preserved from upstream: `i`, `o`, `cr`, `cw` are USD
per million input, output, cache-read and five-minute cache-write tokens. Their
`a` variants apply above 200,000 prompt tokens; `fast` is an explicit multiplier.
One-hour Claude cache creation uses twice the applicable input rate in the
legacy snapshot, matching the upstream ModelRates implementation.

`data/verified-rates.json` overlays six exact model entries verified against
official provider documentation on **2026-09-26**. Every entry records its source
URLs and verification date. Rates below are USD per million tokens at standard
processing rates; input excludes separately counted cache reads and writes.

| Exact model | Input | Output | Cache read | Cache write | One-hour cache write | Fast multiplier |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| `claude-opus-5-5` | 4 | 20 | 0.20 | 5 | 8 | 2 |
| `claude-opus-5` | 5 | 25 | 0.50 | 6.25 | 10 | 2 |
| `claude-fable-5-1` | 10 | 50 | 0.25 | 12.50 | 20 | Unsupported |
| `gpt-6-astra` | 10 | 50 | 1 | 12.50 | Unsupported | 2 |
| `gpt-5.6-luna` | 0.20 | 1.20 | 0.02 | 0.25 | Unsupported | 2 |
| `gpt-6-luna` | 0.10 | 0.50 | 0.01 | 0.125 | Unsupported | 2 |

The three Claude models use these rates throughout their context windows.
Their `cw` rate is for five-minute cache creation; `cwh` explicitly records
the one-hour rate. Fast pricing also multiplies cache rates. Sources:
[Claude pricing](https://platform.claude.com/docs/en/about-claude/pricing),
[Opus 5.5](https://platform.claude.com/docs/en/models/opus-5-5/overview),
[Opus 5](https://platform.claude.com/docs/en/models/opus-5/overview), and
[Fable 5.1](https://platform.claude.com/docs/en/models/fable-5-1/overview).

The three OpenAI models use `long_context_threshold: 272000`. Above 272,000
prompt tokens, the **entire request** costs 2x for input/cache reads/cache
writes and 1.5x for output. Exactly 272,000 remains at standard rates. Fast
mode doubles the applicable rates at either tier. Their `cw` means OpenAI
cache creation, with no assumed five-minute or one-hour TTL; the unsupported
one-hour bucket returns `None`. Sources:
[OpenAI pricing](https://developers.openai.com/api/docs/pricing),
[GPT-6 Astra](https://developers.openai.com/api/docs/models/gpt-6-astra),
[GPT-5.6 Luna](https://developers.openai.com/api/docs/models/gpt-5.6-luna), and
[GPT-6 Luna](https://developers.openai.com/api/docs/models/gpt-6-luna).

Pricing only accepts exact names. Unknown/internal/new models, unsupported fast
rates, invalid token counts, and aggregate usage whose request boundaries are
unknown for a tiered model return `None`. There is no name guessing or fallback
to an unrelated model. Cached Codex input must first be removed from ordinary
input before calling `estimate`; scanner normalization handles this.

The caller must supply complete request token counts to determine context tiers
and the service tier actually used to apply fast pricing. An unknown request
boundary remains unpriced even when the supplied delta is small: a partial
request delta does not establish the full request's context size. OpenAI cache
creation can only be priced when the caller provides its separate token count.
Batch/Flex discounts, regional processing premiums, tool charges and negotiated
rates are not represented by this API.

These are fixed bundled **API-equivalent estimates**, not live prices,
historical invoices, quota consumption, or subscription charges. They are not
automatically refreshed from the network.
