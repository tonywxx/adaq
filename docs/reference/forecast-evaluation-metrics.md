# Forecast Evaluation Metrics

Forecast Evaluation uses only aligned, available predictions and verifiable realized labels. Metrics describe forecast evidence, not Strategy profitability or investment quality. Reports retain unavailable rows and window-level results instead of silently dropping them.

## Common evidence

- **Coverage** is `aligned rows / evaluation rows` in `[0, 1]`.
- **Missingness** is `1 - coverage` in `[0, 1]`.
- **Distribution** records count, minimum, maximum, population mean, and population standard deviation for aligned predictions and realized labels.
- **Time-window stability** partitions evaluation rows into consecutive, non-overlapping windows of the configured Bar count. A final partial window remains visible.

## MAE

Mean Absolute Error is `mean(|prediction - realized|)` in Forecast Target-native units. Its range is `[0, +∞)` and lower is better. Scale depends on the Target and cannot be compared universally across Targets.

## RMSE

Root Mean Squared Error is `sqrt(mean((prediction - realized)²))` in Forecast Target-native units. Its range is `[0, +∞)` and lower is better; squaring gives larger errors more weight.

## Mean bias

Mean bias is `mean(prediction - realized)` with range `(-∞, +∞)`. A value closer to zero means less average signed bias, but positive and negative errors can cancel.

## Pearson correlation

Pearson correlation is `cov(prediction, realized) / (σprediction × σrealized)` in `[-1, 1]`. It is unavailable for fewer than two aligned rows or when either aligned series is constant. Sign and magnitude require research context; no universal quality threshold applies.

## Brier Score

Brier Score is `mean((probability - label)²)` for binary labels zero and one. Its range is `[0, 1]` and lower is better. Interpret it with class balance and calibration evidence; no universal quality threshold applies.

## Log Loss

Log Loss is `-mean(label × ln(p) + (1 - label) × ln(1 - p))`. AdaQ deterministically clips `p` to `[1e-15, 1 - 1e-15]`, giving an approximate range of `[0, 34.539]` and retaining finite evidence for exact zero or one forecasts. Lower is better; confident wrong forecasts receive a larger penalty.

## ROC AUC

ROC AUC is `(concordant positive-negative pairs + 0.5 × tied pairs) / all positive-negative pairs`, with range `[0, 1]`. It is unavailable with typed reason `requires-both-realized-classes` unless both realized classes are present. Higher values indicate stronger ranking separation, not a universal investment-quality threshold.

## Calibration

Calibration uses ten fixed equal-width probability buckets: `[0.0, 0.1)`, ..., `[0.9, 1.0]`. Each bucket records its boundaries, count, mean prediction, and observed positive frequency. Empty buckets remain explicit with unavailable means; small populated buckets remain weak evidence.

## Custom Binary Targets

A Custom Binary Target without a matching host evaluator and verifiable realized labels retains prediction distribution, missingness, provenance, and unavailable-row evidence. AdaQ records no Brier Score, Log Loss, ROC AUC, calibration, or other Target-specific metric claim and reports `requires-verifiable-realized-labels`.

## Versions

- Common coverage: `coverage@1`
- Distribution: `distribution@1`
- Expected Value metrics: `expected-value@1`
- Probability metrics: `binary-probability@1`
- Calibration: `equal-width-10-buckets@1`
- Stability windows: `non-overlapping-windows@1`
