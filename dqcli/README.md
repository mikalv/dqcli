# dqcli (dq)

TUI + NDJSON domain availability checker powered by `librdap-storm` (RDAP-first, WHOIS fallback on unknown).

## Install

```bash
cargo build -p dqcli
```

## Usage

```bash
# TUI (default, prioritizes parsed TLD if FQDN)

# NDJSON stream

# Explicit TLDs
```

## Controls (TUI)
- `Enter` / `y`: copy selected domain
- `o`: open selected domain in browser (Namecheap search)
- `Tab` / `f`: filter All / Available / Taken
- `i`: edit query
- `q` / `Esc`: quit
- Navigation: `↑/↓` or `j/k`, `PgUp/PgDn`, `Home/g`, `End/G`

UI elements: spinner, progress bar (% complete), optional specific-domain row (if FQDN), live results, toast bar.

## NDJSON format

```json
{
  "query": "etellerannetlangtdomene",
  "tld": "com",
  "domain": "etellerannetlangtdomene.com",
  "available": true,
  "status": "available",
  "error": null
}
```
Status: `available | taken | error`.

## Config
`~/.config/dq/config.toml`

```toml
[tlds]
never  = ["xxx", "adult"]
```

## Notes
- Uses `librdap-storm` with shared reqwest pool + per-endpoint rate limiting.
- WHOIS fallback only when RDAP is unknown.
