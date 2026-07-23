# qloxide dev recipes — run `just` (or `just --list`) to see them all.

jq := "jq"
play := "/tmp/qloxide-play"

# List available recipes
default:
    @just --list --unsorted

# ── quality gate ─────────────────────────────────────────────────────

# Everything CI will run: tests, clippy, formatting, supply chain
ci: test lint fmt-check deny

test:
    cargo test
    cargo test --features gen

lint:
    cargo clippy --all-targets -- -D warnings
    cargo clippy --all-targets --all-features -- -D warnings

fmt-check:
    cargo fmt --check

# RUSTSEC advisories, license allowlist, duplicate versions, sources
deny:
    cargo deny check

# ── examples ─────────────────────────────────────────────────────────

# Brent crude portfolio: all four reports
demo:
    cargo run --quiet -- --config examples/brent/brent.toml

# Brent portfolio, selected reports (e.g. `just report pnl positions`)
report +names:
    cargo run --quiet -- --config examples/brent/brent.toml \
        {{ prepend("--report ", names) }}

# Price the example UST bond from JSON
bond:
    cargo run --quiet --example bond_pricing

# Brent Sep26 iron condor vs real ICE settles (data via qloxide-ice) has its
# own justfile with the example: examples/brent-condor/justfile
# (`cd examples/brent-condor && just book` / `just day <date>` / `just series`)
# Brent Dec26 ATM straddle (long-tenor sibling, same layout):
# examples/brent-option/justfile

# ── scenario playground ──────────────────────────────────────────────
# A throwaway copy of the Brent portfolio in {{play}}; mutate its market
# data and reprice. Start with `just play-init`, then chain bumps.

# (Re)create the playground from the example portfolio
play-init:
    rm -rf {{play}}
    cp -r examples/brent {{play}}
    @echo "playground at {{play}} — now try: just play-vol 0.5"

# Reprice the playground (P&L report)
play-pnl:
    cargo run --quiet -- --config {{play}}/brent.toml --report pnl

# Set the option's vol (e.g. `just play-vol 0.5`) and reprice
play-vol vol: && play-pnl
    {{jq}} '.vol_surfaces."ICE-BRN-K26".vol = {{vol}}' \
        {{play}}/market.json > {{play}}/.tmp && mv {{play}}/.tmp {{play}}/market.json

# Move the option's underlying future (e.g. `just play-underlying 78.0`)
play-underlying price: && play-pnl
    {{jq}} '.market_prices."ICE-BRN-K26" = {{price}}' \
        {{play}}/market.json > {{play}}/.tmp && mv {{play}}/.tmp {{play}}/market.json

# Roll the valuation date forward (e.g. `just play-date 2026-03-25`) — watch theta
play-date date: && play-pnl
    {{jq}} '.valuation_date = "{{date}}" | .as_of = "{{date}}T19:30:00Z"' \
        {{play}}/market.json > {{play}}/.tmp && mv {{play}}/.tmp {{play}}/market.json

# Delete the vol surface: load warns, option shows UNPRICED, totals warn
play-break-vol: && play-pnl
    {{jq}} 'del(.vol_surfaces)' \
        {{play}}/market.json > {{play}}/.tmp && mv {{play}}/.tmp {{play}}/market.json

# Remove the playground
play-clean:
    rm -rf {{play}}
