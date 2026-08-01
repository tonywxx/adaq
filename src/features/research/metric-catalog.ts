export type MetricDefinition = {
	id: string;
	version: string;
	label: string;
	meaning: string;
	formula: string;
	direction: string;
	range?: string;
	caveat: string;
	undefinedState: string;
	documentationUrl: string;
};

const REFERENCE =
	"https://github.com/tonywxx/adaq/blob/main/docs/reference/research-metrics.md";

const metric = <Id extends string>(
	id: Id,
	definition: Omit<MetricDefinition, "id" | "version" | "documentationUrl">,
) => ({
	...definition,
	id,
	version: "1.0.0",
	documentationUrl: `${REFERENCE}#${id.replaceAll(".", "-")}`,
});

export const METRIC_CATALOG = {
	"forecast.aligned-count": metric("forecast.aligned-count", {
		label: "Aligned predictions",
		meaning:
			"Forecast rows with both an available prediction and a verifiable realized label.",
		formula: "count(aligned prediction-label rows)",
		direction:
			"More aligned evidence usually improves precision, but quality still depends on provenance and representativeness.",
		range: "Integers in [0, evaluation row count]",
		caveat:
			"A large sample does not prove that evidence is out-of-sample or representative.",
		undefinedState: "Always defined; zero means no rows could be evaluated.",
	}),
	"forecast.coverage": metric("forecast.coverage", {
		label: "Coverage",
		meaning:
			"Share of evaluation rows with aligned, evaluable forecast evidence.",
		formula: "aligned rows / evaluation rows",
		direction: "Higher means fewer evaluation rows were unavailable.",
		range: "[0, 1]",
		caveat:
			"Coverage measures availability, not prediction quality or Strategy profitability.",
		undefinedState:
			"Defined as zero when the evaluation contains no aligned rows.",
	}),
	"forecast.missingness": metric("forecast.missingness", {
		label: "Missingness",
		meaning:
			"Share of evaluation rows without aligned, evaluable forecast evidence.",
		formula: "1 - coverage",
		direction: "Lower means fewer unavailable evaluation rows.",
		range: "[0, 1]",
		caveat:
			"Inspect retained unavailable-row reasons; missing evidence may be systematic.",
		undefinedState: "Defined from coverage for every report.",
	}),
	"forecast.mae": metric("forecast.mae", {
		label: "MAE",
		meaning: "Mean absolute forecast error in Target-native units.",
		formula: "mean(|prediction - realized|)",
		direction: "Lower indicates smaller average absolute error.",
		range: "[0, +∞)",
		caveat:
			"Scale depends on the Target; MAE is Model prediction quality, not Strategy profitability.",
		undefinedState:
			"Unavailable without aligned, verifiable continuous realized labels.",
	}),
	"forecast.rmse": metric("forecast.rmse", {
		label: "RMSE",
		meaning: "Root mean squared forecast error in Target-native units.",
		formula: "sqrt(mean((prediction - realized)²))",
		direction: "Lower indicates smaller error; large errors receive more weight.",
		range: "[0, +∞)",
		caveat: "Scale depends on the Target; RMSE is not Strategy profitability.",
		undefinedState:
			"Unavailable without aligned, verifiable continuous realized labels.",
	}),
	"forecast.mean-bias": metric("forecast.mean-bias", {
		label: "Mean bias",
		meaning: "Average signed forecast error.",
		formula: "mean(prediction - realized)",
		direction: "Closer to zero indicates less average signed bias.",
		range: "(-∞, +∞)",
		caveat:
			"Positive and negative errors can cancel; no universal quality threshold applies.",
		undefinedState:
			"Unavailable without aligned, verifiable continuous realized labels.",
	}),
	"forecast.pearson-correlation": metric("forecast.pearson-correlation", {
		label: "Pearson correlation",
		meaning:
			"Linear association between aligned predictions and realized labels.",
		formula: "cov(prediction, realized) / (σprediction × σrealized)",
		direction: "Interpret sign and magnitude in the research context.",
		range: "[-1, 1]",
		caveat:
			"Correlation is not profitability, causality, or a universal quality score.",
		undefinedState:
			"Unavailable with fewer than two aligned rows or when either series is constant.",
	}),
	"forecast.brier-score": metric("forecast.brier-score", {
		label: "Brier Score",
		meaning:
			"Mean squared error between a probability forecast and its binary realized label.",
		formula: "mean((probability - label)²)",
		direction: "Lower indicates smaller probability error.",
		range: "[0, 1]",
		caveat:
			"Interpret with class balance and calibration; no universal quality threshold applies.",
		undefinedState: "Unavailable without aligned binary realized labels.",
	}),
	"forecast.log-loss": metric("forecast.log-loss", {
		label: "Log Loss",
		meaning: "Mean binary cross-entropy of probability forecasts.",
		formula: "-mean(label×ln(p) + (1-label)×ln(1-p))",
		direction: "Lower is better; confident errors receive a larger penalty.",
		range: "Approximately [0, 34.539] after clipping p to [1e-15, 1-1e-15]",
		caveat:
			"Interpret with class balance; no universal quality threshold applies.",
		undefinedState: "Unavailable without aligned binary realized labels.",
	}),
	"forecast.roc-auc": metric("forecast.roc-auc", {
		label: "ROC AUC",
		meaning:
			"Probability that a positive label ranks above a negative label, with ties worth one half.",
		formula:
			"(concordant positive-negative pairs + 0.5×ties) / all positive-negative pairs",
		direction: "Higher indicates stronger ranking separation.",
		range: "[0, 1]",
		caveat:
			"Class balance and use context matter; no universal investment-quality threshold applies.",
		undefinedState: "Unavailable unless both realized classes are present.",
	}),
	"forecast.calibration": metric("forecast.calibration", {
		label: "Calibration",
		meaning:
			"Agreement between mean forecast probability and observed positive frequency in ten fixed buckets.",
		formula: "for each bucket, mean(probability) compared with mean(label)",
		direction: "Closer agreement indicates better calibration.",
		range: "Both bucket means are in [0, 1]",
		caveat:
			"Empty buckets remain explicit and small populated buckets are weak evidence.",
		undefinedState:
			"A bucket mean is unavailable when that bucket has no aligned rows.",
	}),
	"forecast.pearson-ic": metric("forecast.pearson-ic", {
		label: "Time-series Pearson IC",
		meaning:
			"Linear association between Score predictions and realized Targets in one single-Instrument time-series.",
		formula: "cov(score, target) / (σscore × σtarget)",
		direction: "Interpret sign and magnitude in the research context.",
		range: "[-1, 1]",
		caveat:
			"This is not cross-sectional IC, Strategy profitability, or a universal quality score.",
		undefinedState:
			"Unavailable with fewer than two aligned rows or when Score or Target is constant.",
	}),
	"forecast.spearman-rank-ic": metric("forecast.spearman-rank-ic", {
		label: "Time-series Spearman Rank IC",
		meaning:
			"Rank association between Score predictions and realized Targets in one Instrument, preserving ties.",
		formula: "Pearson correlation of deterministic average ranks",
		direction: "Interpret sign and magnitude in the research context.",
		range: "[-1, 1]",
		caveat:
			"This is not future cross-sectional IC, profitability, or a universal quality score.",
		undefinedState:
			"Unavailable with fewer than two aligned rows or when either ranked series is constant.",
	}),
	"forecast.window-icir": metric("forecast.window-icir", {
		label: "Window ICIR",
		meaning:
			"Mean ordered-window Pearson IC divided by its population standard deviation.",
		formula: "mean(valid window IC) / population σ(valid window IC)",
		direction:
			"Interpret only with ordered window values and their sample count.",
		range: "(-∞, +∞)",
		caveat:
			"Single-Instrument stability evidence; not Strategy profitability, turnover, or a universal score.",
		undefinedState:
			"Unavailable unless at least two valid window IC values exist and vary.",
	}),
	"forecast.five-quantiles": metric("forecast.five-quantiles", {
		label: "Five quantiles",
		meaning:
			"Realized Target evidence grouped by ascending Score, keeping tied Scores together.",
		formula: "five deterministic rank buckets",
		direction: "Inspect monotonicity and every bucket's sample count.",
		range: "Five explicit buckets; some may be empty",
		caveat:
			"Descriptive single-Instrument evidence only; not portfolio return or a trading recommendation.",
		undefinedState:
			"Buckets may be empty when evidence is sparse or ties cross bucket boundaries.",
	}),
	"strategy.total-return": metric("strategy.total-return", {
		label: "Total return",
		meaning:
			"Strategy equity change over the Backtest period relative to initial equity.",
		formula: "final equity / initial equity - 1",
		direction: "Higher indicates greater simulated return for this period.",
		range: "[-1, +∞)",
		caveat:
			"Historical simulated profitability depends on the period, costs, execution assumptions, and risk.",
		undefinedState:
			"Unavailable when a Backtest does not produce valid initial and final equity.",
	}),
	"strategy.cagr": metric("strategy.cagr", {
		label: "CAGR",
		meaning:
			"Annualized compounded Strategy equity growth over the Backtest duration.",
		formula: "(final equity / initial equity)^(1 / years) - 1",
		direction: "Higher indicates greater annualized simulated growth.",
		range: "[-1, +∞)",
		caveat:
			"Annualizing short or unrepresentative periods can mislead; inspect drawdown and evidence length.",
		undefinedState:
			"Reported as zero when duration or positive final equity cannot support compounding.",
	}),
	"strategy.max-drawdown": metric("strategy.max-drawdown", {
		label: "Max drawdown",
		meaning:
			"Largest peak-to-trough decline in Strategy equity during the inspected period.",
		formula: "minimum over time of (equity / prior peak equity - 1)",
		direction: "Closer to zero indicates a smaller historical equity decline.",
		range: "[-1, 0]",
		caveat:
			"Observed drawdown is path- and period-dependent and does not bound future loss.",
		undefinedState: "Unavailable without an equity curve.",
	}),
	"strategy.sharpe": metric("strategy.sharpe", {
		label: "Sharpe",
		meaning:
			"Annualized excess mean Strategy return per unit of annualized return volatility.",
		formula: "(annualized mean return - risk-free rate) / annualized volatility",
		direction:
			"Higher indicates more simulated excess return per unit of measured volatility.",
		range: "(-∞, +∞)",
		caveat:
			"Sensitive to period, sampling, return distribution, and risk-free assumption; no universal threshold applies.",
		undefinedState: "AdaQ reports zero when return volatility is zero.",
	}),
	"strategy.sortino": metric("strategy.sortino", {
		label: "Sortino",
		meaning:
			"Annualized excess mean Strategy return per unit of annualized downside deviation.",
		formula:
			"(annualized mean return - risk-free rate) / annualized downside deviation",
		direction:
			"Higher indicates more simulated excess return per unit of measured downside variation.",
		range: "(-∞, +∞)",
		caveat:
			"Sensitive to period, sample size, and the chosen downside definition; no universal threshold applies.",
		undefinedState: "AdaQ reports zero when downside deviation is zero.",
	}),
	"strategy.excess-return": metric("strategy.excess-return", {
		label: "Excess return",
		meaning: "Strategy total return minus the frozen benchmark total return.",
		formula: "Strategy total return - benchmark total return",
		direction:
			"Positive means the simulated Strategy outperformed this benchmark over this period.",
		range: "(-∞, +∞)",
		caveat:
			"Depends on benchmark choice, period, fees, and execution assumptions.",
		undefinedState:
			"Unavailable without both Strategy and benchmark equity evidence.",
	}),
	"strategy.final-equity": metric("strategy.final-equity", {
		label: "Final equity",
		meaning: "Simulated ending Strategy equity in quote-asset units.",
		formula: "final cash + final base quantity × final price",
		direction: "Interpret relative to initial equity and risk taken.",
		range: "[0, +∞) quote-asset units",
		caveat:
			"A nominal amount is not comparable across initial allocations, quote assets, or periods.",
		undefinedState: "Unavailable when the final portfolio cannot be valued.",
	}),
	"strategy.realized-pnl": metric("strategy.realized-pnl", {
		label: "Realized P&L",
		meaning:
			"Sum of simulated profit and loss realized by fills, in quote-asset units.",
		formula: "sum(fill realized P&L)",
		direction:
			"Positive is profit and negative is loss for the realized portion.",
		range: "(-∞, +∞) quote-asset units",
		caveat: "Excludes the value change of the remaining open base position.",
		undefinedState: "Defined as zero when no fill realizes profit or loss.",
	}),
	"strategy.unrealized-pnl": metric("strategy.unrealized-pnl", {
		label: "Unrealized P&L",
		meaning:
			"Value change of the remaining simulated base position at the final price.",
		formula: "final base quantity × final price - remaining cost basis",
		direction:
			"Positive is an unrealized gain and negative is an unrealized loss.",
		range: "(-∞, +∞) quote-asset units",
		caveat: "Depends on the final mark price and is not locked in by a fill.",
		undefinedState: "Defined as zero when no open base position remains.",
	}),
	"strategy.total-fees": metric("strategy.total-fees", {
		label: "Total fees",
		meaning: "Sum of simulated execution fees in quote-asset units.",
		formula: "sum(fill fee)",
		direction:
			"Lower means less simulated fee drag for otherwise comparable evidence.",
		range: "[0, +∞) quote-asset units",
		caveat:
			"Depends on the frozen fee schedule, fill policy, turnover, and market path.",
		undefinedState: "Defined as zero when no fee-bearing fill occurs.",
	}),
	"strategy.win-rate": metric("strategy.win-rate", {
		label: "Win rate",
		meaning:
			"Share of realized outcomes with positive P&L among non-zero realized outcomes.",
		formula: "winning realized outcomes / non-zero realized outcomes",
		direction:
			"Higher means a greater share of realized outcomes were profitable.",
		range: "[0, 1]",
		caveat:
			"Ignores win and loss size; a high win rate can coexist with negative total P&L.",
		undefinedState:
			"AdaQ reports zero when there are no non-zero realized outcomes.",
	}),
	"strategy.fill-count": metric("strategy.fill-count", {
		label: "Fills",
		meaning: "Number of simulated order fills recorded by the Backtest.",
		formula: "count(fills)",
		direction:
			"Descriptive only; neither higher nor lower is inherently favorable.",
		range: "Integers in [0, +∞)",
		caveat:
			"Fill count is not trade count and depends on execution and partial-fill behavior.",
		undefinedState: "Defined as zero when no order is filled.",
	}),
	"strategy.realized-trade-count": metric("strategy.realized-trade-count", {
		label: "Trades",
		meaning: "Count of non-zero simulated realized P&L outcomes.",
		formula: "count(fill realized P&L ≠ 0)",
		direction:
			"Descriptive only; neither higher nor lower is inherently favorable.",
		range: "Integers in [0, +∞)",
		caveat:
			"This engine-level count may differ from position- or round-trip trade conventions.",
		undefinedState: "Defined as zero when no fill realizes profit or loss.",
	}),
	"execution.order-count": metric("execution.order-count", {
		label: "Orders",
		meaning: "Number of simulated Orders recorded by the Backtest.",
		formula: "count(orders)",
		direction:
			"Descriptive only; neither higher nor lower is inherently favorable.",
		range: "Integers in [0, +∞)",
		caveat: "Order count depends on Strategy decisions and execution policy.",
		undefinedState: "Defined as zero when no Order is recorded.",
	}),
	"execution.order-quantity": metric("execution.order-quantity", {
		label: "Order quantity",
		meaning: "Base-asset quantity requested by a simulated Order.",
		formula: "requested base quantity",
		direction:
			"Descriptive only; interpret with price, equity, and Instrument units.",
		range: "[0, +∞) base-asset units",
		caveat: "Requested quantity may differ from filled quantity.",
		undefinedState: "Unavailable when an Order has no valid requested quantity.",
	}),
	"execution.limit-price": metric("execution.limit-price", {
		label: "Limit price",
		meaning: "Quote-asset price limit attached to a simulated Order.",
		formula: "Order limit price",
		direction: "Descriptive only; interpret with Order side and market evidence.",
		range: "[0, +∞) quote-asset units per base unit",
		caveat:
			"A limit price does not prove that an Order was filled at that price.",
		undefinedState: "Unavailable when an Order has no valid limit price.",
	}),
	"execution.fill-quantity": metric("execution.fill-quantity", {
		label: "Filled quantity",
		meaning: "Base-asset quantity executed by a simulated Fill.",
		formula: "executed base quantity",
		direction: "Descriptive only; compare with requested quantity.",
		range: "[0, +∞) base-asset units",
		caveat: "One Order may produce multiple partial Fills.",
		undefinedState: "Unavailable when a Fill has no valid executed quantity.",
	}),
	"execution.requested-quantity": metric("execution.requested-quantity", {
		label: "Requested quantity",
		meaning: "Original base-asset quantity requested by the Fill's Order.",
		formula: "originating Order requested quantity",
		direction: "Descriptive only; compare with filled quantity.",
		range: "[0, +∞) base-asset units",
		caveat: "It can exceed one Fill's quantity when execution is partial.",
		undefinedState: "Unavailable without the originating Order quantity.",
	}),
	"execution.fill-price": metric("execution.fill-price", {
		label: "Fill price",
		meaning: "Quote-asset price per base unit for a simulated Fill.",
		formula: "Fill quote value / filled base quantity",
		direction: "Descriptive only; interpret with side and market evidence.",
		range: "[0, +∞) quote-asset units per base unit",
		caveat: "Execution assumptions determine the simulated Fill price.",
		undefinedState: "Unavailable when a Fill has no valid price.",
	}),
	"execution.fill-fee": metric("execution.fill-fee", {
		label: "Fill fee",
		meaning: "Simulated execution fee charged for one Fill.",
		formula: "Fill quote value × frozen fee rate",
		direction: "Lower means less fee drag for otherwise identical execution.",
		range: "[0, +∞) quote-asset units",
		caveat: "Depends on the frozen fee schedule, role, quantity, and price.",
		undefinedState: "Defined as zero when the Fill has no fee.",
	}),
	"execution.fill-realized-pnl": metric("execution.fill-realized-pnl", {
		label: "Fill realized P&L",
		meaning: "Profit or loss realized by one simulated Fill.",
		formula:
			"closed quantity × (Fill price - cost basis), signed by position side",
		direction: "Positive is realized profit and negative is realized loss.",
		range: "(-∞, +∞) quote-asset units",
		caveat: "Excludes unrealized value change in any remaining position.",
		undefinedState: "Defined as zero when the Fill closes no position quantity.",
	}),
	"validation.completed": metric("validation.completed", {
		label: "Completed",
		meaning:
			"Validation windows or market contexts that completed with metric evidence.",
		formula: "count(validation items without failure)",
		direction:
			"More completed items provide broader retained evidence, subject to design quality.",
		range: "Integers in [0, configured validation items]",
		caveat:
			"Completion does not imply favorable results or independent evidence.",
		undefinedState: "Defined as zero when no validation item completes.",
	}),
	"validation.failed": metric("validation.failed", {
		label: "Failed",
		meaning:
			"Validation windows or market contexts retained with failure evidence.",
		formula: "configured validation items - completed items",
		direction: "Lower means fewer configured items failed to produce metrics.",
		range: "Integers in [0, configured validation items]",
		caveat:
			"Inspect each retained failure; failures must not be silently excluded from conclusions.",
		undefinedState: "Defined as zero when every validation item completes.",
	}),
	"validation.total-fees": metric("validation.total-fees", {
		label: "Total fees",
		meaning:
			"Sum of simulated execution fees across completed sample-in and sample-out Runs or completed market contexts.",
		formula: "sum(completed validation Run total fees)",
		direction: "Descriptive only; compare only equivalent Validation designs.",
		range: "[0, +∞) quote-asset units",
		caveat:
			"Overlapping windows can count the same period more than once; fee schedules and completion failures affect the total.",
		undefinedState: "Defined as zero when no completed Run records a fee.",
	}),
	"validation.realized-trade-count": metric("validation.realized-trade-count", {
		label: "Trades",
		meaning:
			"Sum of non-zero simulated realized P&L outcomes across completed sample-in and sample-out Runs or market contexts.",
		formula: "sum(completed validation Run realized trade count)",
		direction:
			"Descriptive only; neither higher nor lower is inherently favorable.",
		range: "Integers in [0, +∞)",
		caveat:
			"Overlapping windows can count the same period more than once, and this engine-level count is not a round-trip convention.",
		undefinedState: "Defined as zero when no completed Run realizes P&L.",
	}),
	"validation.average-sample-out-return": metric(
		"validation.average-sample-out-return",
		{
			label: "Average sample-out return",
			meaning:
				"Arithmetic mean Strategy total return across completed sample-out windows or market contexts.",
			formula: "sum(completed sample-out total returns) / completed count",
			direction: "Higher indicates greater average simulated held-out return.",
			range: "[-1, +∞)",
			caveat:
				"An equal-window average is not compounded performance and failures remain separate evidence.",
			undefinedState: "AdaQ reports zero when no validation item completes.",
		},
	),
	"validation.average-sample-in-return": metric(
		"validation.average-sample-in-return",
		{
			label: "Average sample-in return",
			meaning:
				"Arithmetic mean Strategy total return across completed sample-in windows.",
			formula: "sum(completed sample-in total returns) / completed count",
			direction:
				"Use as context for comparing sample-out behavior, not as a goal by itself.",
			range: "[-1, +∞)",
			caveat:
				"Sample-in performance is not held-out evidence; cross-market validation reports zero because it has no sample-in leg.",
			undefinedState: "AdaQ reports zero when no sample-in result exists.",
		},
	),
	"validation.worst-sample-out-drawdown": metric(
		"validation.worst-sample-out-drawdown",
		{
			label: "Worst drawdown",
			meaning:
				"Most negative maximum drawdown among completed sample-out windows or market contexts.",
			formula: "minimum(completed sample-out max drawdown)",
			direction:
				"Closer to zero indicates a smaller worst observed held-out decline.",
			range: "[-1, 0]",
			caveat:
				"Observed validation drawdown does not bound future loss and depends on the selected windows or markets.",
			undefinedState: "AdaQ reports zero when no validation item completes.",
		},
	),
	"validation.average-sample-out-sharpe": metric(
		"validation.average-sample-out-sharpe",
		{
			label: "Average Sharpe",
			meaning:
				"Arithmetic mean Backtest Sharpe ratio across completed sample-out windows or market contexts.",
			formula: "sum(completed sample-out Sharpe) / completed count",
			direction:
				"Higher indicates greater average measured risk-adjusted simulated return.",
			range: "(-∞, +∞)",
			caveat:
				"Averaging ratios hides dispersion; inspect every window and do not apply a universal threshold.",
			undefinedState: "AdaQ reports zero when no validation item completes.",
		},
	),
	"validation.cross-market-return-spread": metric(
		"validation.cross-market-return-spread",
		{
			label: "Total return spread",
			meaning:
				"Difference between the highest and lowest completed Strategy total return across validation markets.",
			formula:
				"max(completed market total return) - min(completed market total return)",
			direction:
				"Lower indicates less observed cross-market dispersion, not necessarily better profitability.",
			range: "[0, +∞)",
			caveat:
				"Historical market selection and sample size determine this dispersion; it is not a guarantee.",
			undefinedState: "Unavailable unless at least two market contexts complete.",
		},
	),
} as const;

export type MetricId = keyof typeof METRIC_CATALOG;

export function getMetricDefinition(id: string): MetricDefinition {
	const definition = METRIC_CATALOG[id as MetricId];
	if (!definition) throw new Error(`Missing Metric Definition: ${id}`);
	return definition;
}
