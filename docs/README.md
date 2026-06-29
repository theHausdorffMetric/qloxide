# qloxide docs

Design, planning, and review notes for qloxide. These were carried over from the
original `ql` scaffold mono-repo (now retired); the canonical, shipped design
rationale lives in [`../ARCHITECTURE.md`](../ARCHITECTURE.md).

## Upstream reference

qloxide is a from-scratch Rust reimplementation inspired by — and deliberately
diverging from — **QuantMath**:

- **Repo:** <https://github.com/MarcusRainbow/QuantMath.git>
- **Pinned reference commit:** `b51ffac` (2020-05-28)
- **Author:** Marcus Rainbow · **License:** MIT

See [`quantmath-upstream.md`](quantmath-upstream.md) for notes on the upstream
library, and `../ARCHITECTURE.md` §1.3 / §9 for the point-by-point rationale on
where and why qloxide departs from it.

## Contents

| Doc | What | Status |
|-----|------|--------|
| [`quantmath-upstream.md`](quantmath-upstream.md) | Architecture notes on the upstream QuantMath library (the reference qloxide reimplements) | Reference |
| [`options-plan.md`](options-plan.md) | Options-on-futures implementation plan (Black76, Phase 1) with reference test values | Active plan |
| [`qloxide-review-2026-06-12.md`](qloxide-review-2026-06-12.md) | Status review + design/code evaluation; improvement Phases A & B completed, Phase C pending | Live (Phase C open) |
| [`qloxide-architecture.md`](qloxide-architecture.md) | Build status, layer/pricing build order, open items (mono-repo logistics sections are now historical) | Status / partly historical |
| [`qloxide-build-plan.md`](qloxide-build-plan.md) | Original incremental build plan | Historical (superseded) |
| [`opt_src/`](opt_src/) | Reference option-pricing implementations (gbsm, bachelier, asian, mathutils) cited by `options-plan.md` for tested values | Reference code |
