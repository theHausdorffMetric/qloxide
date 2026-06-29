# Upstream QuantMath — Reference Notes

These notes describe the **original QuantMath** Rust library that qloxide is a
from-scratch reimplementation of (and deliberately diverges from — see
[`../ARCHITECTURE.md`](../ARCHITECTURE.md) §1.3 "Why not use QuantMath directly?").

- **Upstream repo:** <https://github.com/MarcusRainbow/QuantMath.git>
- **Pinned reference commit:** `b51ffac` (2020-05-28, "Merge pull request #52")
- **Author:** Marcus Rainbow · **License:** MIT

> Carried over from the retired `ql` scaffold mono-repo, where QuantMath was
> vendored as a read-only submodule for reference.

---

## Project Overview

QuantMath is a Rust financial mathematics library for risk-neutral pricing and risk, designed for integration into investment bank/hedge fund infrastructure. It provides both a native Rust library and C FFI bindings (via cbindgen). Author: Marcus Rainbow. License: MIT.

## Build & Test Commands

All commands run from `QuantMath/` directory:

```bash
cargo build --verbose --all       # Build all targets
cargo test --verbose --all        # Run all tests
cargo test <test_name>            # Run a single test by name
cargo test -- --nocapture         # Run tests with stdout visible
cargo clippy                      # Lint
rustfmt src/**/*.rs               # Format
```

The crate produces both `lib` (Rust library) and `cdylib` (C dynamic library) outputs.

C header regeneration: `cbindgen` generates `quantmath.h` (configured in `cbindgen.toml`).

C++ integration tests are in `src/cpp-test/` and require building the dynamic library first, then compiling with g++.

## Architecture

Strict layered module hierarchy with no backward dependencies (each module only depends on modules below it in this list):

1. **core** — Error types (`qm.rs`), polymorphic serialization factories (`factories.rs`), DAG deduplication (`dedup.rs`)
2. **math** — Black-Scholes formulae, interpolation, Brent root-finding, numerics
3. **dates** — Explicit date handling (not year-fractions), holiday calendars, business day rules, day counts
4. **data** — Market data inputs (vol surfaces, yield curves, dividend streams, spot prices) and bump definitions for risk
5. **instruments** — Financial products (options, bonds, baskets, assets, currencies). Some self-price; some price via Monte Carlo paths
6. **risk** — Market data bumping, dependency tracking, risk reports (delta/gamma, vega/volga, theta). Cache system reuses unchanged calculations across bumps
7. **models** — Stochastic models (Black diffusion, etc.) for price evolution
8. **pricers** — Pricing engines: Monte Carlo (path averaging) and self-pricer (instrument-intrinsic pricing)
9. **solvers** — Implied volatility solver and 1D solver trait
10. **facade** — IT-facing interface: data-driven JSON API (`handle.rs`) and C FFI bindings (`c_interface.rs`)

## Key Design Patterns

- **Data-driven API**: Extensive use of `serde`/`serde_tagged`/`erased_serde` for JSON serialization. IT systems interact via JSON without rebuilding when instruments/models are added.
- **Polymorphic type registries**: `factories.rs` and `lazy_static` registries enable extensible instrument/model/risk types through tagged serialization.
- **Bump caching**: The risk cache system (`risk/cache.rs`) reuses unaffected calculations when bumping market data — critical for performance in multi-underlying exotics.
- **Lifecycle rigor**: Instruments model explicit flows, ex-dates, and settlement dates. Nothing disappears; flows split and track ownership transitions.
- **Recursive instruments**: Baskets can contain composites/quantos with dynamic indices, forming recursive product hierarchies.

## Adding New Components

Adding a new instrument, risk type, or model should typically require changes to only one file plus registering in the relevant `mod.rs`.

## Dependencies

Key crates: `statrs` (statistics), `ndarray` (N-dimensional arrays), `nalgebra` (linear algebra), `rand` (RNG for Monte Carlo), `serde`/`serde_json`/`serde_tagged`/`erased_serde` (serialization), `libc` (C FFI).

## Test Data

JSON test fixtures for the C++ test runner live in `src/cpp-test/inputs/` (currencies, instruments, market data, fixings, pricer config, expected reports).
