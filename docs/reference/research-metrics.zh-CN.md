# 研究指标目录

这是 Forecast Evaluation Report、Backtest Run 和 Validation Report 当前显示的全部指标的简体中文参考。桌面 UI 使用相同的稳定 ID 和 1.0.0 版本定义。

Forecast 指标描述 Model 预测质量；Strategy 指标描述模拟收益、风险与执行；Validation 指标汇总保留的 Strategy 证据。任何指标都不是通用投资质量阈值或交易建议。

## Forecast Evaluation

| 稳定 ID 与版本 | 定义 |
| --- | --- |
| <a id="forecast-aligned-count"></a>forecast.aligned-count@1.0.0 | **已对齐预测。** 同时具有可用预测和可验证真实标签的 Forecast 行数。公式：对齐的预测-标签行计数。更多证据通常提高估计精度，但样本量不能证明证据为样本外或具有代表性。范围：从 0 到评估总行数的整数。始终有定义；0 表示没有可评估行。 |
| <a id="forecast-coverage"></a>forecast.coverage@1.0.0 | **覆盖率。** 具有对齐证据的评估行占比。公式：对齐行数 / 评估行数。越高表示不可用行越少。范围：[0, 1]。它衡量可用性，不衡量预测质量或 Strategy 收益。没有对齐行时定义为 0。 |
| <a id="forecast-missingness"></a>forecast.missingness@1.0.0 | **缺失率。** 没有对齐证据的评估行占比。公式：1 - 覆盖率。越低表示不可用行越少。范围：[0, 1]。应检查保留的不可用原因，因为缺失可能具有系统性。每份报告都由覆盖率确定。 |
| <a id="forecast-mae"></a>forecast.mae@1.0.0 | **MAE。** 以 Target 原生单位表示的平均 Forecast 绝对误差。公式：mean(abs(prediction - realized))。越低表示平均绝对误差越小。范围：[0, +无穷)。尺度取决于 Target；这是 Model 预测质量，不是 Strategy 收益。没有对齐且可验证的连续标签时不可用。 |
| <a id="forecast-rmse"></a>forecast.rmse@1.0.0 | **RMSE。** 以 Target 原生单位表示的 Forecast 均方根误差。公式：sqrt(mean((prediction - realized)^2))。越低表示误差越小，且大误差权重更高。范围：[0, +无穷)。尺度取决于 Target；它不是 Strategy 收益。没有对齐且可验证的连续标签时不可用。 |
| <a id="forecast-mean-bias"></a>forecast.mean-bias@1.0.0 | **平均偏差。** Forecast 有符号误差的平均值。公式：mean(prediction - realized)。越接近 0 表示平均有符号偏差越小。范围：(-无穷, +无穷)。正负误差可能抵消，因此不存在通用质量阈值。没有对齐且可验证的连续标签时不可用。 |
| <a id="forecast-pearson-correlation"></a>forecast.pearson-correlation@1.0.0 | **Pearson 相关系数。** 对齐预测与真实标签的线性关联。公式：covariance(prediction, realized) / (预测标准差 x 标签标准差)。范围：[-1, 1]；符号和大小须结合研究语境解释。它不代表收益、因果关系或通用评分。少于两行或任一序列为常数时不可用。 |
| <a id="forecast-brier-score"></a>forecast.brier-score@1.0.0 | **Brier Score。** 概率预测与二元标签之间的均方误差。公式：mean((probability - label)^2)。越低表示概率误差越小。范围：[0, 1]。须结合类别平衡和校准解释，不存在通用阈值。没有对齐的二元标签时不可用。 |
| <a id="forecast-log-loss"></a>forecast.log-loss@1.0.0 | **Log Loss。** 平均二元交叉熵。公式：-mean(label x ln(p) + (1-label) x ln(1-p))。越低越好，过度自信的错误惩罚更大。将 p 截断到 [1e-15, 1-1e-15] 后近似范围为 [0, 34.539]。须结合类别平衡解释。没有对齐二元标签时不可用。 |
| <a id="forecast-roc-auc"></a>forecast.roc-auc@1.0.0 | **ROC AUC。** 正类排在负类之前的概率，并将并列计为一半。公式：(一致排序对 + 0.5 x 并列对) / 正负样本对。越高表示排序区分度越强。范围：[0, 1]。不存在通用投资质量阈值。只有两类真实标签都出现时才可用。 |
| <a id="forecast-calibration"></a>forecast.calibration@1.0.0 | **校准。** 十个固定区间内平均预测概率与实际正类频率的一致程度。公式：逐区间比较 mean(probability) 与 mean(label)。两者范围均为 [0, 1]，越接近越好。空区间会明确保留，小样本区间证据较弱。区间没有对齐行时其均值不可用。 |
| <a id="forecast-pearson-ic"></a>forecast.pearson-ic@1.0.0 | **时间序列 Pearson IC。** 单一 Instrument 时间序列中 Score 与真实 Target 的线性关联。公式：covariance(score, target) / (Score 标准差 x Target 标准差)。范围：[-1, 1]；须结合语境解释。它不是横截面 IC、Strategy 收益或通用评分。少于两行或任一序列为常数时不可用。 |
| <a id="forecast-spearman-rank-ic"></a>forecast.spearman-rank-ic@1.0.0 | **时间序列 Spearman Rank IC。** 单一 Instrument 中 Score 与真实 Target 的秩关联，并保留并列值。公式：确定性平均秩的 Pearson 相关系数。范围：[-1, 1]；须结合语境解释。它不是未来横截面 IC 或收益。少于两行或任一秩序列为常数时不可用。 |
| <a id="forecast-window-icir"></a>forecast.window-icir@1.0.0 | **窗口 ICIR。** 有序窗口 Pearson IC 均值除以其总体标准差。公式：mean(valid window IC) / population standard deviation(valid window IC)。范围：(-无穷, +无穷)；须同时检查每个窗口和样本数。这是单一 Instrument 稳定性证据，不是收益或换手率。至少需要两个有效且不同的窗口 IC。 |
| <a id="forecast-five-quantiles"></a>forecast.five-quantiles@1.0.0 | **五分位。** 按 Score 升序将真实 Target 分组，并保持相同 Score 在同一组。公式：五个确定性秩分组。应检查单调性和每组样本数；方向没有通用优劣。五组明确保留，部分可为空。这是单一 Instrument 描述性证据，不是组合收益。 |

## Backtest Strategy

| 稳定 ID 与版本 | 定义 |
| --- | --- |
| <a id="strategy-total-return"></a>strategy.total-return@1.0.0 | **总收益率。** Backtest 期间 Strategy 权益的相对变化。公式：final equity / initial equity - 1。越高表示模拟收益越大。范围：[-1, +无穷)。结果取决于期间、成本、执行假设和风险。没有有效的初始及最终权益时不可用。 |
| <a id="strategy-cagr"></a>strategy.cagr@1.0.0 | **CAGR。** Strategy 权益的年化复合增长。公式：(final equity / initial equity)^(1 / years) - 1。越高表示年化模拟增长越大。范围：[-1, +无穷)。短期样本年化可能误导。期间或正最终权益不足以支持复合计算时 AdaQ 报告 0。 |
| <a id="strategy-max-drawdown"></a>strategy.max-drawdown@1.0.0 | **最大回撤。** Strategy 权益最大的峰谷跌幅。公式：全时段 equity / prior peak equity - 1 的最小值。越接近 0 表示观察到的跌幅越小。范围：[-1, 0]。它依赖路径和期间，不能限制未来损失。没有权益曲线时不可用。 |
| <a id="strategy-sharpe"></a>strategy.sharpe@1.0.0 | **Sharpe。** 年化 Strategy 超额平均收益与年化收益波动率之比。公式：(annualized mean return - risk-free rate) / annualized volatility。越高表示每单位波动对应的测得超额模拟收益越大。范围：(-无穷, +无穷)。期间、采样、分布和无风险利率假设都会影响结果，不存在通用阈值。波动率为 0 时 AdaQ 报告 0。 |
| <a id="strategy-sortino"></a>strategy.sortino@1.0.0 | **Sortino。** 年化 Strategy 超额平均收益与年化下行偏差之比。公式：(annualized mean return - risk-free rate) / annualized downside deviation。越高表示每单位下行偏差对应的测得超额模拟收益越大。范围：(-无穷, +无穷)。期间、样本量和下行定义都会影响结果。下行偏差为 0 时 AdaQ 报告 0。 |
| <a id="strategy-excess-return"></a>strategy.excess-return@1.0.0 | **超额收益率。** Strategy 总收益率减去冻结基准收益率。公式：Strategy total return - benchmark total return。正值表示该期间模拟跑赢基准。范围：(-无穷, +无穷)。结果取决于基准、期间、费用和执行假设。缺少任一权益序列时不可用。 |
| <a id="strategy-final-equity"></a>strategy.final-equity@1.0.0 | **最终权益。** 以报价资产单位表示的 Strategy 模拟期末权益。公式：final cash + final base quantity x final price。应相对初始权益和承担的风险解释。范围：[0, +无穷) 报价资产单位。不同初始配置、报价资产或期间的名义金额不可直接比较。无法估值组合时不可用。 |
| <a id="strategy-realized-pnl"></a>strategy.realized-pnl@1.0.0 | **已实现 P&L。** 模拟 Fill 实现的损益总和。公式：sum(fill realized P&L)。正值为盈利，负值为亏损。范围：(-无穷, +无穷) 报价资产单位。不包含剩余仓位价值变化。没有 Fill 实现损益时定义为 0。 |
| <a id="strategy-unrealized-pnl"></a>strategy.unrealized-pnl@1.0.0 | **未实现 P&L。** 剩余模拟基础资产仓位的价值变化。公式：final base quantity x final price - remaining cost basis。正值为浮盈，负值为浮亏。范围：(-无穷, +无穷) 报价资产单位。它依赖最终标记价格，尚未由成交锁定。没有未平仓位时定义为 0。 |
| <a id="strategy-total-fees"></a>strategy.total-fees@1.0.0 | **总费用。** 模拟执行费用总和。公式：sum(fill fee)。在其他证据可比时，越低表示费用拖累越小。范围：[0, +无穷) 报价资产单位。费用表、Fill 策略、换手与市场路径都会影响结果。没有产生费用的 Fill 时定义为 0。 |
| <a id="strategy-win-rate"></a>strategy.win-rate@1.0.0 | **胜率。** 非零已实现结果中正收益结果的占比。公式：winning outcomes / non-zero outcomes。越高表示盈利结果占比越大。范围：[0, 1]。它忽略盈亏幅度，因此高胜率仍可能对应负 P&L。没有非零已实现结果时 AdaQ 报告 0。 |
| <a id="strategy-fill-count"></a>strategy.fill-count@1.0.0 | **Fills。** 模拟订单成交记录数。公式：count(fills)。仅作描述，方向没有固有优劣。范围：非负整数。Fill 数不是交易数，并受部分成交行为影响。没有订单成交时定义为 0。 |
| <a id="strategy-realized-trade-count"></a>strategy.realized-trade-count@1.0.0 | **Trades。** 非零模拟已实现 P&L 结果数。公式：count(fill realized P&L 不等于 0)。仅作描述。范围：非负整数。该引擎级计数可能不同于往返交易口径。没有 Fill 实现损益时定义为 0。 |

### 执行证据

| 稳定 ID 与版本 | 定义 |
| --- | --- |
| <a id="execution-order-count"></a>execution.order-count@1.0.0 | **Orders。** 模拟 Order 数。公式：count(orders)。仅作描述。范围：非负整数。它取决于 Strategy 决策和执行策略。没有记录 Order 时定义为 0。 |
| <a id="execution-order-quantity"></a>execution.order-quantity@1.0.0 | **Order 数量。** 请求的基础资产数量。公式：requested base quantity。应结合价格、权益和 Instrument 单位解释。范围：非负基础资产单位。它可能不同于成交数量。缺少有效数量时不可用。 |
| <a id="execution-limit-price"></a>execution.limit-price@1.0.0 | **限价。** Order 指定的报价资产价格界限。公式：Order limit price。应结合买卖方向和市场证据解释。范围：每基础资产单位的非负报价资产单位。它不能证明成交价格。缺少有效价格时不可用。 |
| <a id="execution-fill-quantity"></a>execution.fill-quantity@1.0.0 | **成交数量。** 单个 Fill 执行的基础资产数量。公式：executed base quantity。应与请求数量比较。范围：非负基础资产单位。一个 Order 可能产生多个部分 Fill。缺少有效数量时不可用。 |
| <a id="execution-requested-quantity"></a>execution.requested-quantity@1.0.0 | **请求数量。** Fill 所属 Order 的原始请求数量。公式：originating Order quantity。应与成交数量比较。范围：非负基础资产单位。它可能大于一个部分 Fill。缺少原始数量时不可用。 |
| <a id="execution-fill-price"></a>execution.fill-price@1.0.0 | **成交价格。** 单个 Fill 每基础资产单位的报价资产价格。公式：Fill quote value / filled quantity。应结合方向和市场证据解释。范围：每基础资产单位的非负报价资产单位。执行假设会影响结果。缺少有效价格时不可用。 |
| <a id="execution-fill-fee"></a>execution.fill-fee@1.0.0 | **Fill 费用。** 单个 Fill 的模拟费用。公式：Fill quote value x frozen fee rate。在其他执行相同时越低表示拖累越小。范围：非负报价资产单位。费用表、角色、数量和价格都会影响结果。没有费用时定义为 0。 |
| <a id="execution-fill-realized-pnl"></a>execution.fill-realized-pnl@1.0.0 | **Fill 已实现 P&L。** 单个 Fill 实现的盈利或亏损。公式：closed quantity x (Fill price - cost basis)，并按持仓方向确定符号。正值为盈利，负值为亏损。范围：任意报价资产金额。不包含剩余未实现 P&L。没有平仓数量时定义为 0。 |

## Validation

| 稳定 ID 与版本 | 定义 |
| --- | --- |
| <a id="validation-completed"></a>validation.completed@1.0.0 | **已完成。** 成功产生指标证据的 Validation 窗口或市场数。公式：count(items without failure)。更多已完成项扩大保留证据，但不代表结果有利或相互独立。范围：0 到配置项总数。始终有定义；0 表示均未完成。 |
| <a id="validation-failed"></a>validation.failed@1.0.0 | **失败。** 保留了失败证据的 Validation 窗口或市场数。公式：configured items - completed items。越低表示失败越少。范围：0 到配置项总数。应检查每个失败，而不是从结论中排除。全部完成时定义为 0。 |
| <a id="validation-total-fees"></a>validation.total-fees@1.0.0 | **总费用。** 已完成样本内、样本外 Run 或市场的费用总和。公式：sum(completed validation Run fees)。仅作描述。范围：非负报价资产单位。重叠窗口可能重复计算同一期间；费用表和失败项也会影响总数。没有已完成 Run 记录费用时定义为 0。 |
| <a id="validation-realized-trade-count"></a>validation.realized-trade-count@1.0.0 | **Trades。** 已完成样本内、样本外 Run 或市场的非零已实现 P&L 结果总数。公式：sum(completed validation Run trade count)。仅作描述。范围：非负整数。重叠窗口可能重复计算同一期间，且此口径不是往返交易定义。没有已完成 Run 实现 P&L 时定义为 0。 |
| <a id="validation-average-sample-out-return"></a>validation.average-sample-out-return@1.0.0 | **平均样本外收益率。** 已完成留出窗口或市场的 Strategy 总收益率算术平均。公式：sum(completed sample-out returns) / completed count。越高表示平均模拟收益越大。范围：[-1, +无穷)。它不是复合收益，失败项仍是独立证据。没有完成项时 AdaQ 报告 0。 |
| <a id="validation-average-sample-in-return"></a>validation.average-sample-in-return@1.0.0 | **平均样本内收益率。** 已完成样本内窗口的 Strategy 总收益率算术平均。公式：sum(completed sample-in returns) / completed count。应用作比较语境。范围：[-1, +无穷)。它不是留出证据；跨市场 Validation 没有样本内部分。没有样本内结果时 AdaQ 报告 0。 |
| <a id="validation-worst-sample-out-drawdown"></a>validation.worst-sample-out-drawdown@1.0.0 | **最差回撤。** 已完成留出窗口或市场中最负的最大回撤。公式：minimum(completed sample-out max drawdown)。越接近 0 表示观察到的最差跌幅越小。范围：[-1, 0]。选择范围决定观察风险，且不能限制未来损失。没有完成项时 AdaQ 报告 0。 |
| <a id="validation-average-sample-out-sharpe"></a>validation.average-sample-out-sharpe@1.0.0 | **平均 Sharpe。** 已完成留出窗口或市场的 Backtest Sharpe 算术平均。公式：sum(completed sample-out Sharpe) / completed count。越高表示平均测得风险调整模拟收益越大。范围：(-无穷, +无穷)。比率平均会隐藏离散程度，应检查每项且不使用通用阈值。没有完成项时 AdaQ 报告 0。 |
| <a id="validation-cross-market-return-spread"></a>validation.cross-market-return-spread@1.0.0 | **总收益率极差。** Validation 市场中最高与最低已完成 Strategy 收益率之差。公式：max(completed market return) - min(completed market return)。越低表示观察到的跨市场离散越小，不一定表示收益更好。范围：[0, +无穷)。市场选择与样本量会影响结果。至少两个市场完成时才可用。 |

Validation 证据还显示 strategy.total-return、strategy.max-drawdown 和 strategy.sharpe，并复用上文完全相同的定义。
