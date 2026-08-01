# 外部 Kronos Adapter

[English](README.md) | [简体中文](README.zh-CN.md)

本参考路径在研究者自行管理的 Python 环境中同时运行 `Kronos-small` 与 `Kronos-Tokenizer-base`，并把生成的 OHLCV 路径转换为规范的 AdaQ Forecast Signals。Tokenizer 只负责编码和解码 K 线，本身不是推理模型；自回归推理由 `Kronos-small` 完成。

AdaQ 桌面进程不会加载 Python、PyTorch、Hugging Face 代码、下载的权重或任意外部代码。Adapter 只写出受限的 `.adaq-signals` archive；AdaQ 校验后将其作为 **Externally Generated** 证据导入。它不是 Verified inference，也不具备 Marketplace 发布资格。

## 受支持环境与精确上游输入

本文档支持 64 位 macOS、Linux 或 Windows 上的 Python 3.10–3.12，并要求显式选择 CPU、CUDA 或 MPS；GPU 不是必需条件。创建隔离环境并安装固定版本依赖：

```sh
cd examples/external-models/kronos
python3.12 -m venv .venv
. .venv/bin/activate
python -m pip install --upgrade pip
python -m pip install -r requirements.txt
```

获取以下精确上游 revision：

| Artifact | 官方来源 | Revision |
| --- | --- | --- |
| Kronos 源码及预处理 | `https://github.com/shiyu-coder/Kronos` | `67b630e67f6a18c9e9be918d9b4337c960db1e9a` |
| `Kronos-small` 模型与权重 | `NeoQuasar/Kronos-small` | `901c26c1332695a2a8f243eb2f37243a37bea320` |
| `Kronos-Tokenizer-base` 与权重 | `NeoQuasar/Kronos-Tokenizer-base` | `0e0117387f39004a9016484a186a908917e22426` |

```sh
git clone https://github.com/shiyu-coder/Kronos.git upstream/Kronos
git -C upstream/Kronos checkout 67b630e67f6a18c9e9be918d9b4337c960db1e9a
hf download NeoQuasar/Kronos-small --revision 901c26c1332695a2a8f243eb2f37243a37bea320 --local-dir artifacts/Kronos-small
hf download NeoQuasar/Kronos-Tokenizer-base --revision 0e0117387f39004a9016484a186a908917e22426 --local-dir artifacts/Kronos-Tokenizer-base
shasum -a 256 artifacts/Kronos-small/model.safetensors artifacts/Kronos-Tokenizer-base/model.safetensors upstream/Kronos/model/kronos.py
```

使用前须检查 `upstream/Kronos/LICENSE` 以及两个固定 Hugging Face revision 的 `LICENSE`/model card 元数据。它们目前声明 MIT；研究者仍须自行确认实际下载内容的条款及其是否适用于预期用途。Adapter 会把双方的权重与配置文件、上游预处理、Adapter 自身及完整推理/运行时配置的哈希写入 provenance。

## 与 Snapshot 精确对齐的输入

1. 在 AdaQ 中下载并冻结精确的 Market Data Snapshot，再从研究 UI 复制 Snapshot ID。
2. 在 Settings → General 选择 **Open Data Folder**，随后退出 AdaQ，使外部只读进程读取稳定的数据库与 Parquet 文件。
3. 如有需要，查询拥有该 Snapshot 的 User ID：

   ```sh
   sqlite3 /path/to/data/adaq.db 'select distinct user_id from market_data_snapshot_access order by user_id;'
   ```

Adapter 以只读模式打开 `adaq.db`，校验 `(User, Snapshot ID)` 访问记录，并读取不可变 metadata 指向的精确 Snapshot Parquet。它不会用最新数据或近似匹配的序列替代。

## Forecast 配置与确定性 Seed

在本目录运行：

```sh
python adapter.py \
  --database /path/to/data/adaq.db \
  --user-id YOUR_USER_ID \
  --snapshot-id YOUR_SNAPSHOT_ID \
  --kronos-root upstream/Kronos \
  --model artifacts/Kronos-small \
  --tokenizer artifacts/Kronos-Tokenizer-base \
  --lookback 400 \
  --horizons 1,6,24 \
  --temperature 1.0 --top-k 0 --top-p 0.9 --sample-count 1 \
  --seed 7 --device cpu \
  --output kronos-small.adaq-signals
```

`lookback` 不得超过 Kronos-small 的 512 Bar 上下文。Adapter 在有序推理前将 Seed 应用于 Python、NumPy、PyTorch 和全部 CUDA device，并连同 temperature、top-k、top-p、sample count、device、revision 与哈希一起记录。不同硬件或上游库的外部 GPU kernel 仍可能产生差异，这也是结果保持 Externally Generated 的原因之一。比较重复运行时应保留环境与输出哈希。

## 规范转换规则

Adapter 为 Snapshot 中每个 Closed Bar 保留原始行身份，并输出一个有序 Signal frame：

| 字段 | 规则 |
| --- | --- |
| Prediction Time | 来源 Closed Bar 的收盘边界，Unix 毫秒 |
| Signal 名称 | 声明 horizon `H` 对应 `expected-close-return-H-bar` |
| 聚合 | `generated_close[H - 1] / origin_close - 1` |
| 单位与 Value Scale | 小数收益率（`0.01` 表示 1%），native scale |
| Forecast Target | 精确 `H` Bars 的内置连续 `future-close-return` |
| `availableAt` | `closed-bar@1` 下等于 Prediction Time；Strategy 最早只能在下一 Bar 执行 |
| Warmup | 连续 Closed Bars 少于 `lookback` 时为 unavailable |
| Bar Gap | 开始新上下文并重启 Warmup；上下文不得跨越缺口 |
| MissingInput | 缺少所需生成路径时为 unavailable；不得填零或移动 horizon |

生成的未来 OHLCV 行不会作为已实现 market data 导入；只有聚合后的 Forecast Signals 进入 `signals.parquet`。

规范 manifest 记录精确 Model Artifact 身份、模型与 Tokenizer 权重哈希及 revision、Adapter 与预处理哈希、完整推理配置、Seed、上游来源、许可说明、availability policy、Snapshot 和 Producer Segment。若无法获得精确的上游训练、拟合或验证边界，则明确记录 unknown，绝不推断未知事实。

## 导入、检查、评估与 Backtest

1. 重新打开 AdaQ，在 **Models → Create Dataset** 导入 `kronos-small.adaq-signals`。
2. 在 **Signal Datasets** 确认精确 Snapshot、Signal contract、行状态、哈希、Producer Segment、provenance 及 **Externally Generated** trust state。
3. 在 **Evaluation Reports** 选择兼容的 Expected Value signal 与评估窗口，创建不可变 Forecast Evaluation Report，检查 coverage/missingness 与 Expected Value 指标，并导出 JSON 或 Markdown。
4. `Unknown` Evaluation Evidence State 表示上游训练、拟合或 normalization window 证据不完整。它可用于本地研究，但不代表 out-of-sample、Verified 或任何表现保证。
5. 导入或选择 Forecast Signal Slot 在 Prediction Kind、Target、horizon 与 native scale 上完全匹配的 Strategy。在 **Backtest** 中选择同一 Snapshot 并绑定导入的 Dataset signal。
6. 运行 Dataset-first Backtest。AdaQ 不会隐式重跑 Kronos。检查 Run 冻结的 Feature Plan、Dataset Lock、Producer provenance、Unknown evidence state、pauses、decisions、fills 与 metrics。

Forecast Evaluation 评估预测；Backtest 及后续 Validation Report 评估 Strategy 行为；二者都不会改变外部推理的 trust state。

## 确定性自动化 fixture

小 fixture 使用已记录的生成路径，不下载权重，也不需要 GPU：

```sh
python -m unittest test_adapter.py
python adapter.py --snapshot-json fixtures/snapshot.json --fixture-paths fixtures/generated-paths.json \
  --lookback 2 --horizons 1,2 --seed 7 --output /tmp/kronos-fixture.adaq-signals
```

Rust 测试使用同一 fixture 覆盖转换 literal、manifest、Parquet、archive import、Unknown 评估证据、兼容 Strategy 导入、Dataset-first Backtest 与 Run Dataset Lock。

## 可选真实推理证据与故障排查

由于下载、内存、运行时间与加速器支持各不相同，真实推理是可选路径。重试或报告故障前须保存以下证据：

- 下载：命令、URL/repository、精确 revision、HTTP/错误输出、文件大小与 SHA-256。
- 运行或内存：OS、CPU/GPU、加速器/运行时版本、`python --version`、`python -m pip freeze`、命令、Seed/配置、峰值内存、耗时与完整 traceback。
- Schema 或转换：Snapshot ID、User ID（对外发送时脱敏）、instrument/interval、Bar 数量/缺口、horizons/lookback、出错 Prediction Time 与生成路径 shape；除非明确需要，不要附带私有 market data。
- 导入：`.adaq-signals` SHA-256/大小、检查敏感路径后的 manifest、AdaQ 精确错误，以及是否出现任何 Dataset。
- 评估：Dataset ID、signal/horizon/window、已创建时的 Report ID、Evidence State、unavailable rows 与导出/错误。
- Backtest：Strategy package hash、Snapshot/Dataset/signal binding、preflight 错误或 Run ID、Dataset Lock、pauses 与相关导出。

常见修复：Snapshot 不匹配时必须从该精确 Snapshot 重新生成；`warmup` 需要更长的连续 segment 或更小且有记录的 lookback；`missing-input` 需要完整生成路径；hash/schema 错误应保留失败 archive 并重新生成，不得手工修改；内存不足可减小 lookback/sample count 或改用 CPU，并把变化后的配置记录为新证据。

## 未来 Qlib 边界

未来可由 Microsoft Qlib 准备训练数据、训练或微调 Artifact，再通过相同 External Model Adapter 边界导出预测。M8 不包含 Qlib 集成、训练、受控 Python Runner、Verified external inference、live execution 或 Marketplace 发布。未来受控 Runner 或经验证的 AdaQ Model Component 必须增加自己的执行证据，同时保留 `.adaq-signals` provenance 与 Dataset-first 边界。
