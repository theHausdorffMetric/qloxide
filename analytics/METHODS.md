# Methods — option-implied risk-neutral densities

The methodology this crate implements, distilled to what the code needs
and self-contained by design. The extended source — the survey that
settled these choices, the failure-mode taxonomy, and the annotated
bibliography — is the `RND/` OKF bundle in the private `ideas` repo
(knowledge layer; it deliberately does not ship with this crate).

Everything below works in **forward (undiscounted) space**: with
`C̃ = e^{rT}·C` the discount factor cancels out of the extraction
entirely, and the martingale condition reads `E[S] = F`.

## 1. The identity, and why it is ill-posed

Breeden–Litzenberger (1978): the risk-neutral density is the second
strike-derivative of the (undiscounted) call curve,

```
q(K) = ∂²C̃/∂K² ,      survival function  P(S > K) = −∂C̃/∂K ,
CDF via the digital identity  F(K) = 1 + C̃'(K).
```

Differentiation is noise-amplifying: price noise of amplitude `ε` on a
strike grid of spacing `ΔK` puts a floor of

```
sd(q̂) ≈ √6 · ε / ΔK²        (forward space)
```

on any finite-difference density estimate. On real settle data this
floor can exceed the peak density itself — which is why the constrained
smoother below is necessary, not optional. Unconstrained smoothers
violate no-arbitrage shape constraints on **more than half of trading
days** (Aït-Sahalia & Duarte 2003); a negative density is *exactly* a
butterfly arbitrage, so positivity must be built in, not checked after.

## 2. Input slice (`slice.rs`)

The input is a single-expiry set of discrete quotes `(K_i, C̃_i)` — the
`SmileSlice`. When built from a calibrated `qloxide::VolSurface::Grid`,
only the grid **nodes** are used: the grid interpolates linearly in vol
between nodes, so sampling anywhere else would inject artificial kinks
exactly where the density takes its second derivative. Node vols are
re-priced to forward Black-76 calls (`r = 0` in the pricer ⇒ discount
factor 1). When a raw-quote sidecar is present (tier-1 data), its quotes
construct the same `SmileSlice` and everything downstream is unchanged.

## 3. Pre-filter diagnostics (`prefilter.rs`)

Discrete no-arbitrage conditions on the raw quotes, reported (never
silently repaired):

- positivity: `C̃_i > 0`;
- monotonicity: call spreads `(C̃_{i+1} − C̃_i)/(K_{i+1} − K_i) ∈ [−1, 0]`
  (the digital price bound in forward space);
- convexity: butterflies `C̃_{i−1} − 2C̃_i + C̃_{i+1} ≥ 0` (scaled for
  uneven grids) — a violated butterfly *is* a locally negative density;
- the `√6·ε/ΔK²` noise floor above, with `ε` = half the price tick.

## 4. Fengler (2009) constrained QP (`fengler.rs`)

Fit a **natural cubic smoothing spline in call-price space** — not IV
space. Non-negativity of the density is convexity of the call curve, and
convexity is a *linear* constraint on a spline's second derivative; this
is what makes a hard guarantee reachable by convex QP.

Decision variables are stacked `x = (g', γ')'` where `g` are call values
at the strike knots `u₁ < … < u_n` (spacings `h_i`) and `γ` their second
derivatives. With weights `W = diag(w_i)` and the Reinsch matrices `Q`
(second-difference) and `R` (h/6-tridiagonal), the paper's eqs (15)–(18),
verified against the published text:

```
min_x  −y'x + ½ x'Bx     s.t.  A'x = 0                             (15)
       A = (Q, −R'),  B = diag(W, λR),  y = (w₁y₁,…,w_n y_n, 0,…)'

γ_i ≥ 0,  i = 2,…,n−1   (γ₁ = γ_n = 0: natural spline)             (16)

(g₂−g₁)/h₁ − (h₁/6)γ₂ ≥ −1                                         (17)
(g_n−g_{n−1})/h_{n−1} + (h_{n−1}/6)γ_{n−1} ≤ 0

F − u₁ ≤ g₁ ≤ F,   g_n ≥ 0                                         (18)
```

The equality `A'x = 0` is the Reinsch value–second-derivative
consistency relation `Q'g = Rγ`; `B ≻ 0`, so the minimiser is unique.
(17) constrains the spline's *boundary derivatives* — hence the `h/6`
correction terms, not plain difference quotients; with convexity already
enforced, the two end slopes are necessary and sufficient. In forward
space the paper's discount/dividend factors in (17)–(18) reduce to 1 and
the spot bound becomes the forward `F`. Calendar constraints (the
paper's eq (19), iterating maturities last-to-first under a dominance
condition) are **not implemented** — the crate currently fits single
tenors.

**Why the guarantee is global:** a natural cubic spline has a
piecewise-*linear* second derivative, so `γ_i ≥ 0` at the knots implies
`C̃'' ≥ 0` everywhere between them. No post-hoc checking, no gaps
between nodes.

Implementation notes that matter:

- **Solver:** clarabel (interior-point conic QP, pure Rust). Interior
  point is the right family here — Fengler's convexity constraints are
  active over wide strike ranges, exactly where ADMM-style solvers give
  inaccurate multipliers.
- **Tolerances:** clarabel's default gap tolerance (1e-8) leaves
  complementarity residuals that put spurious duals on wing `γ ≥ 0`
  constraints (slack ~1e-5), visibly bending the wings; tolerances are
  tightened to 1e-12.
- **λ selects smoothness, not moments.** Moments barely move across
  λ ∈ [1e-5, 1e1] (the active convexity constraints regularise them),
  but small λ leaves tick-quantised quotes as a spike *comb* in the
  pointwise density, and large λ costs repricing error. Default λ = 1.

## 5. Density extraction (`density.rs`)

From the fitted spline: density = `γ` (piecewise linear between knots),
CDF via the digital identity `F(K) = 1 + C̃'(K)`, and moments computed
**exactly** from the piecewise representation (no quadrature error).
Certificates carried with the result: the `γ ≥ 0` guarantee, repricing
residuals vs the input quotes, and `E[S]` vs `F`.

## 6. GPD tails (`tails.rs`)

The interior fit only covers the listed strike range; tail mass beyond
it needs a parametric tail. Choices, per the survey:

- **GPD over GEV**: two parameters instead of three — the location is
  *not free* (it is the attachment strike) and both families share the
  same asymptotic shape `ξ`.
- **Pasting criterion: repricing, not density value-matching**
  (Bollinger–Melick–Thomas 2023). The attached tail must correctly
  *price the options at the strikes it replaces* — an integral condition
  on the whole tail, categorically stronger than the local smoothness
  conditions of Figlewski-style (9a)–(9c) value-matching.

Construction, right tail (left mirrored on `c − S`, puts via parity):

```
P(S > s) = p · z(s)^(−1/ξ),   z(s) = 1 + ξ(s−c)/σ
C̃(K)    = p·σ·z(K)^(1−1/ξ) / (1−ξ),   K ≥ c,  ξ < 1
```

- tail mass `p` = the interior CDF's mass beyond the attachment `c`
  (exact, by construction);
- `σ` pinned by C¹ price continuity at the splice (the slope matches
  automatically because the slope *is* the tail mass);
- `ξ` fitted by 1-D least squares repricing the observed quotes beyond
  the attachment, excluding tick-floor quotes (< 4·tick-eps).

Mass conservation + C¹ continuity at both splices provably force
`E[S] = F` **exactly** — the martingale condition holds by construction,
not approximately. Density continuity at the splice is deliberately
*not* imposed; the jump is reported as a diagnostic.

**Lee cross-check.** Lee (2004): total implied variance grows at most
linearly in |log-moneyness| with slope ≤ 2, and the wing slope maps to
the number of finite moments — hence to an implied tail index comparable
with the fitted GPD `ξ`. Divergence is *flagged, never auto-resolved*:
on settle data with flat wing IVs the wing slope is biased thin, and the
alarm is the deliverable. Kurtosis is divergent for `ξ ≥ ¼` and reported
as such rather than hidden.

## 7. Perturbation harness (`perturb.rs`)

Bliss–Panigirtzoglou (2002) stability testing: perturb every quote by
uniform ±½-tick noise, refit the *entire* pipeline N times, and report
the empirical 5–95% band on every published statistic — plus a cross-day
stability table. Interpretation, reproduced on our own contracts:
mean is degenerate at `F` (structural, see §6); **sd and quantiles are
publishable** (tight bands); **skew is fragile** (day-to-day moves often
inside their own noise band); **kurtosis is unidentifiable** on
fat-tailed days; **tail shape sign (`ξ > 0`) is robust**. Publish
statistics only with their bands.

## 8. Deliberately not implemented (and where to look)

- **Calendar constraints / multi-tenor** — Fengler eq (19); needed only
  when a multi-tenor grid becomes an input.
- **Cohen–Reisinger–Wang (2020) arbitrage-repair LP** — preprocessor for
  *raw* quote data; today's inputs are calibrated surfaces that already
  pass the prefilter.
- **Smooth-density upgrades** (Fengler–Hin B-splines; stochastic
  collocation) — only if pointwise density smoothness beyond λ-tuning is
  ever needed; moments don't require it.
- **Figlewski GEV value-matching** — superseded by the BMT repricing
  criterion above.

## External validation

Compared against the Minneapolis Fed's market-based probability
densities (independent implementation: Shimko-style cubic B-spline in
IV space, 5-day traded-quote window, linear extrapolation) on WTI over
Jan–Apr 2020 — the COVID crash, and the Fed series' final months (their
oil MPD was discontinued the week WTI settled negative; a ln-return
pipeline cannot represent that regime). On calm dates the distribution
bodies agree to within a percentage point: ±20% move probabilities
within 0.1–0.8pp, decile spreads within 3%. Divergences are
sign-consistent with method: this pipeline enforces `E[S] = F` exactly
and carries crash mass in the GPD tails, so its median sits below and
its left tail above the Fed's — most visibly at peak stress, where the
Fed's published moments imply a mean several percent off the settle
(trade-window pooling + non-enforced martingale). Full study: the RND
bundle's log, 2026-07-26.

## References

- Breeden, D.T., R.H. Litzenberger. 1978. "Prices of State-Contingent
  Claims Implicit in Option Prices." *J. Business* 51(4): 621–651.
  doi:10.1086/296025
- Aït-Sahalia, Y., J. Duarte. 2003. "Nonparametric option pricing under
  shape restrictions." *J. Econometrics* 116(1–2): 9–47.
  doi:10.1016/S0304-4076(03)00102-7
- Fengler, M.R. 2009. "Arbitrage-free smoothing of the implied
  volatility surface." *Quantitative Finance* 9(4): 417–428.
  doi:10.1080/14697680802595585
- Bollinger, T.R., W.R. Melick, C.P. Thomas. 2023. "Principled pasting:
  attaching tails to risk-neutral probability density functions
  recovered from option prices." *Quantitative Finance* 23(12):
  1751–1768. doi:10.1080/14697688.2023.2272677
- Lee, R.W. 2004. "The Moment Formula for Implied Volatility at Extreme
  Strikes." *Mathematical Finance* 14(3): 469–480.
  doi:10.1111/j.0960-1627.2004.00200.x
- Bliss, R.R., N. Panigirtzoglou. 2002. "Testing the stability of
  implied probability density functions." *J. Banking & Finance*
  26(2–3): 381–422. doi:10.1016/S0378-4266(01)00227-8
- Cohen, S.N., C. Reisinger, S. Wang. 2020. "Detecting and Repairing
  Arbitrage in Traded Option Prices." *Applied Mathematical Finance*
  27(5): 345–373. doi:10.1080/1350486X.2020.1846573
- Surveys: Jackwerth 2004 (*Option-Implied Risk-Neutral Distributions
  and Risk Aversion*, Research Foundation of AIMR); Figlewski 2018
  ("Risk-Neutral Densities: A Review," *Annu. Rev. Financ. Econ.* 10:
  329–359, doi:10.1146/annurev-financial-110217-022944).
