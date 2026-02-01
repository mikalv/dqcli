# dq (dqcli)

TUI and NDJSON domain availability checker powered by `librdap-storm` (endpoint-centric RDAP + WHOIS fallback).

The TUI provides a similar experience as the website [instantdomainsearch.com](https://instantdomainsearch.com).

## Misc Information

- Binary: `dq`
- CLI crate: `dqcli`
- Library: `librdap-storm`

## Install

```bash
cargo build -p dqcli         # build dq
cargo build -p librdap-storm # build the library
```

## CLI (dq)

```bash
# TUI (default) – parses FQDN and prioritizes that TLD
dq etellerannetlangtdomene.com

# NDJSON stream
dq etellerannetlangtdomene.com --ndjson | jq .

# Explicit TLDs (overrides auto)
```

### Controls (TUI)
- `Enter` / `y`: copy selected domain
- `o`: open selected domain in browser (Namecheap search)
- `Tab` / `f`: filter All / Available / Taken
- `i`: edit query
- `q` / `Esc`: quit

### Config
`~/.config/dq/config.toml`

```toml
[tlds]
always = ["com", "io", "dev"]
never  = ["xxx", "adult"]
```

## Library (librdap-storm)

```rust
use librdap_storm::{Prober, probe, Availability};

// one domain
let r = probe("example.com").await;

// many domains (streaming)
let prober = Prober::new();
let domains = ["foo.com", "foo.io", "foo.dev"];
let mut stream = prober.probe_stream(domains.into_iter().map(String::from));
while let Some(res) = stream.next().await {
    println!("{} -> {:?}", res.domain, res.availability);
}
```

### Why librdap-storm is awesome
- RDAP-first, WHOIS fallback only on unknown → fast path is lightweight
- Shared HTTP pool + per-endpoint token buckets → scales to 100–1000 req/s without tripping 429s
- Endpoint-centric scheduler streams results as they complete (no batching pauses)
- IANA bootstrap keeps endpoints fresh; built-in list as safety net
- Minimal API: `probe` for one, `probe_stream` for many

### Design highlights
- Shared reqwest client with aggressive pooling
- Per-endpoint token bucket (governor) to avoid 429s
- IANA bootstrap for RDAP endpoints
- WHOIS fallback only on Unknown
- Streaming scheduler: groups by endpoint, buffer_unordered for throughput

## Project layout

```
dqcli/            # dq TUI (binary)
```

## Development

- Build TUI: `cargo build -p dqcli`
- Run TUI help: `cargo run -p dqcli -- --help`
- Build lib: `cargo build -p librdap-storm`

## Status

- New library architecture (endpoint-centric, rate-limited)
- dq renamed from instantdomainsearch
- TUI adds spinner, progress bar, filters, clipboard copy, browser-open, toast
