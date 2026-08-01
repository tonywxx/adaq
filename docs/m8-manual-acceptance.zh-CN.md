# M8 人工验收（macOS ARM64）

这是 M8 的标准人工复核路径。每次只执行表中一行，失败时保留指定证据。Forecast Evaluation 衡量预测证据；Backtest 和 Validation 衡量 Strategy 行为。它们都不代表盈利承诺、Live Trading、Verified external inference 或 Marketplace 审批。

<!-- m8-acceptance:prerequisites -->
## 1. 前提条件、登录与存储

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在仓库根目录运行 `pnpm install --frozen-lockfile`、`rustup toolchain install stable`、`rustup target add --toolchain stable wasm32-unknown-unknown`、`cargo install cargo-component --locked` 和 `cargo install --force --path src-tauri/crates/adaq-component-tooling`。 | Node 依赖和 stable Rust/Component 工具链安装完成；`adaq-component --help` 列出 `new`、`build` 和 `verify`。 | 失败命令、完整输出、`node --version`、`pnpm --version` 和 `rustc --version --verbose`。 |
| 在版本控制之外配置 `VITE_SUPABASE_URL` 与 `VITE_SUPABASE_PUBLISHABLE_KEY`，然后运行 `pnpm tauri dev`。 | 桌面登录界面出现，不显示缺少配置提示。 | 原始提示及变量名；绝不记录 value、密码、OTP 或 token。 |
| 用现有测试账户的 email 和 password 登录。 | Dashboard、sidebar 和当前 User 的研究数据出现。 | 可见错误及展开的技术详情，并隐藏秘密。 |
| 打开 **Settings → Data & Storage**，阅读摘要，再选择 **Open Data Folder**，不要执行 reset。 | 当前本地数据数量可读，应用数据目录打开；账户/登录与设备偏好不属于研究数据 reset 范围。 | 截图、可见/技术错误和 OS 版本。 |

Windows 使用 PowerShell、`py -3.12 -m venv .venv`、`.\.venv\Scripts\Activate.ps1` 和 `cargo install --force --path .\src-tauri\crates\adaq-component-tooling`。Linux 使用与 macOS 相同的 POSIX 命令。原生文件选择器与数据目录因平台而异。下面的标准人工复核运行以 macOS ARM64 为准。

<!-- m8-acceptance:components -->
## 2. 编写、构建、验证、导入与检查 Components

在新的空目录创建项目；已提交示例只作为参考，不能替代生成步骤。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `adaq-component new model m8-forecast-model`、`adaq-component new strategy m8-signal-strategy`、`adaq-component new strategy m8-hybrid-strategy` 和 `adaq-component new strategy m8-composed-strategy --template composed`。 | 生成四个具有不同 UUID 的项目；Model、默认 Strategy 与 composed Strategy 使用正确 SDK feature，Manifest 始终为 `kind: model` 或 `kind: strategy`。 | 命令输出及生成的 `Cargo.toml`/`manifest.json`；不得重新生成或缩短 ID。 |
| 在 `m8-forecast-model` 中，用 [`model-close-score`](../examples/components/model-close-score/src/lib.rs) 的确定性、保留 identity 的实现替换 `src/lib.rs`。对每行设置 `let normalized = (row.values.first().copied().unwrap_or_default() / 100.0).sin();`，并返回 `values: vec![normalized / 100.0, (normalized + 1.0) / 2.0, (normalized + 1.0) / 2.0]`。把生成的 output contract 改为三个名称唯一、horizon 为 1 的输出：`expected-return` = Expected Value + `future-close-return` + native scale；`up-probability` = Probability + `future-close-up` + probability scale；`return-score` = Score + `future-close-return` + percentile scale。保留生成的 `componentId`、versions、market `close` Feature Slot、Single-Instrument scope、embedded Artifact SHA-256 与 `warmupBars: 0`；增加值均为 `0..0` 的 Artifact provenance strings `trainingWindow`、`fittingWindow` 和 `normalizationWindow`。 | 一个真实 batch Model 保留 Instrument ID、Prediction Time、行顺序，并按有效 contract 生成有变化、有限且确定的值；完整且不重叠的 provenance 可以生成 Out-of-sample evaluation evidence。 | 完整 source、Manifest 与 verifier typed error；字段结构与 [`model-close-score/manifest.json`](../examples/components/model-close-score/manifest.json) 对照。 |
| 保持 `m8-signal-strategy` 的生成内容不变。在 `m8-hybrid-strategy` 中，在 `forecast-probability` 旁增加 market `close` Feature Slot，绑定两个 index，并仅在 probability 至少为 `0.5` 且 close 为正时输出 `1`。保持 `m8-composed-strategy` 的生成内容不变。 | 三个权威输入图分别推导为 Signal-driven、Hybrid 与 Composed；不存在作者可控制的 Architecture 字段。 | 三份 source/Manifest 与完整 conformance error。 |
| 在每个项目运行 `adaq-component build`，再运行 `adaq-component verify dist/<project-name>-0.1.0.adaq`。 | 每个 self-contained package 的测试、Wasm build、conformance、大小/完整性检查及 verify 均通过。记录每个 archive SHA-256。 | 完整输出、package path/size、Manifest 和 archive SHA-256。 |
| 在 **Components** 中，对 Model 和三个 Strategy 依次使用 **Import Component Package**，然后选择每一项。 | 精确 ID、version、hash、compatibility、Model outputs/Artifact provenance、Strategy Feature Slots 和推导出的 Architecture 可读、可复制。 | 所选文件、可见错误、展开的技术详情、截图和精确 IDs/hashes。 |

<!-- m8-acceptance:native-dataset -->
## 3. 原生 Forecast Signal Dataset

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 **Backtest → Data** 选择 `BTC-USDT`、`1h` 与至少含 100 Bars 的连续区间，选择 **Prepare Snapshot**，保存精确 Snapshot ID。 | 出现用户范围内的不可变 Snapshot，显示 venue、Instrument、interval、range、Bar count、gaps 与 ID。 | 输入、progress/error、已创建时的 Snapshot ID 和展开的技术详情。 |
| 在 **Models → Create Dataset** 选择 `m8-forecast-model` 和上述精确 Snapshot，然后只点击一次 **Create Dataset**。 | native work 前先显示 painted accessible busy state；重复启动被抑制；Attempt 依次 Pending → Running → Completed，并只发布一个 Dataset。 | Attempt ID、status/progress、diagnostic evidence、Model package hash、Snapshot ID、Seed 和 technical error。 |
| 打开 **Signal Datasets**，展开 **Rows** 与 **Provenance**；超过十行时翻一页。 | 可检查并复制 Dataset/Parquet hashes、精确 Snapshot、Feature Plan、Component Lock、三个 Signal contracts、Seed、`verified-package` trust、Artifact、一个 Producer Segment、engine identity、coverage、Present/unavailable rows、`availableAt`、Warmup/MissingInput 与 Bar-Gap 规则。 | Dataset ID、hashes、失败 row/page、截图和技术详情。 |

<!-- m8-acceptance:external-dataset -->
## 4. 外部 Kronos 证据

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 `examples/external-models/kronos` 创建文档规定的 Python 3.10–3.12 环境并运行 `python -m unittest test_adapter.py`。 | 两个确定性 fixture tests 通过，不下载模型且不需要 GPU；golden `.adaq-signals` transformation 保持逐字节稳定。 | Python/platform 版本、`python -m pip freeze`、命令、完整 traceback 和 fixture SHA-256。 |
| 按[外部 Kronos Adapter](../examples/external-models/kronos/README.zh-CN.md)操作：使用第 3 节精确 Snapshot ID、固定的 `Kronos-small` 与 `Kronos-Tokenizer-base` revisions、Seed `7`，并明确选择 CPU/MPS/CUDA；若硬件或网络阻止执行，则完整保留该指南规定的 download/runtime evidence record。 | 生成与 Snapshot 对齐的 `kronos-small.adaq-signals`；或者诚实记录真实 inference 不可用，不声称已经运行。不得把 Tokenizer 当成 inference model。 | 精确 revision/URLs、licences、hashes、runtime/config、Seed、device、peak memory、elapsed time 和完整错误；不记录 credentials/private data。 |
| 在 **Models → Signal Datasets** 选择 **Import .adaq-signals** 并选中生成的 archive，检查后用 **Export .adaq-signals** 保存到新路径。 | Import 完整验证并原子发布精确 external Dataset；证据保持 **Externally Generated**，保留 Producer Segment、Artifact/weight/Tokenizer/Adapter/preprocessing hashes、unknown training evidence、Snapshot alignment、availability policy 与相同 authoritative export identity。 | Archive SHA-256/size、检查过的 Manifest、精确 typed error、是否出现 Dataset、Dataset ID 和 export path/error。 |

<!-- m8-acceptance:evaluation -->
## 5. Expected Value、Probability 与 Score 评估

对原生 Dataset 的 `expected-return`、`up-probability` 和 `return-score` 分别重复下列操作。使用 Dataset coverage 边界，stability window 设为 `20`。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 **Models → Evaluation Reports** 选择 Dataset 与 Signal，输入完整起止毫秒和 `20`，再选择 **Create Report**。 | 不可变 Report 显示共同的 coverage/missingness/distribution/stability；Expected Value 显示 MAE/RMSE/bias/correlation，Probability 显示 Brier/Log Loss/ROC AUC/calibration，Score 显示 time-series Pearson IC/Spearman IC/window IC/ICIR/quantiles。 | Dataset/Signal/horizon/window、typed error、unavailable rows，以及已创建时的 Report ID。 |
| 对每个渲染出的 metric，用 Tab 聚焦相邻 information control，以键盘打开并关闭，然后分别用 click 和 pointer hover 打开。 | 无需只依赖颜色或 hover 即可读取 meaning、formula、interpretation direction、range、caveat、undefined state 和 reference link；预测质量不会被表述成 Strategy profitability。 | Metric label、interaction mode、截图、focus state 和 accessibility-tree text。 |
| 展开每个 Report 的 **Evidence** 与 **Provenance**，再用新文件名选择 **Export JSON** 和 **Export Markdown**。 | Producer-level evidence 与 unavailable results 保持可见；精确 Dataset/Snapshot/Segments/Artifacts/contracts/hashes/trust/versions 被保留；不会覆盖已有文件。 | Report ID、Evidence State、export name、可见/技术错误，以及安全时的 exported file。 |
| 复制 Model project，保留其 `componentId`，把 version 改为 `0.1.1`，把三个 provenance windows 全部改为 `0..9999999999999`；随后 build、verify、import，用同一 Snapshot 生成第二个 Dataset 并创建一个 Report。再用 imported Kronos Dataset 创建一个 Report。 | 原始 native Reports 为 **Out-of-sample**，`0.1.1` Report 为 **Overlapping**，Kronos Report 为 **Unknown**；warning 始终明确，强 metrics 不会升级 trust 或 evidence。 | 两个 Package/Dataset/Report IDs、Segment windows、三种 computed states、warnings 与 provenance JSON。 |

<!-- m8-acceptance:backtests -->
## 6. Signal-driven、Hybrid 与 Composed Backtests

对三个已导入 Strategy 分别执行下列各行；始终使用第 3 节精确 Snapshot，并选择其中的 subset window。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 **Backtest → Strategy** 选择 Strategy。Signal-driven/Hybrid 的 `forecast-probability` 只能绑定兼容的原生 `up-probability` Dataset Signal；Composed Strategy 保持生成的 Market slots。 | Architecture 显示 Signal-driven、Hybrid 或 Composed；不兼容 Dataset Signals 不出现；Backtest 不会调用 Model。 | Strategy/package hash、slot、available candidates、Dataset/Signal contract 与 gate message。 |
| 选择 **Execution**，allocation 设为 `10000`，Seed 为 `48`，使用完整默认 Spot Execution Profile，设置有效 Dataset subset window，选择 **Validate inputs** 并检查 **Authoritative inputs**，再选择 **Run Backtest**。 | 一个确定性不可变 Run 完成。只有 `availableAt <= decisionTime` 才消费 Signal；fill 不早于 next Bar；对齐但不可用的值生成 `Run Pause::MissingInput`。 | Preflight、typed error、Snapshot/Dataset/Signal binding、已创建时的 Run ID，以及 status/pauses。 |
| 检查 **Overview**、**Decisions**、**Execution** 与 **Provenance**，操作所有 metric information controls，再选择 **Use as new configuration**。 | Results、decisions/pauses、orders/fills/fees、精确 Feature Plan、Architecture、Component/Dataset Locks、Evidence State、Producer provenance、engine identities、seed 与 run window 均存在；copy-as-new 不修改历史 Run。 | Run ID、失败 tab/control、截图、复制的值与技术详情。 |
| 在 **Validation** 选择每个 completed Run，确认它能创建新的 immutable Protocol，且不改变 Snapshot 或 Signal evidence。 | Signal-driven、Hybrid 和 Composed Runs 均可作为 Validation evidence 复用，并保留原始不可变 identity。 | Run/Protocol ID、Snapshot ID、mismatch message 和技术详情。 |

<!-- m8-acceptance:negative-paths -->
## 7. 必须执行的负向路径

只操作 disposable package/archive 副本或新 Attempt；不得编辑 finalized evidence。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 disposable Model Manifest 中把 `horizonBars` 设为 `0` 并运行 `adaq-component build`；恢复后把 Probability 改为 `future-close-return`，再次 build。 | 两种无效 Package contract 都在 import 前以稳定 technical evidence 失败；没有 package 被接受。 | Mutated Manifest、命令、完整 typed error 和 `dist/` 是否变化。 |
| 复制有效 `.adaq-signals`，加入意外 ZIP entry 或修改 `signals.parquet`，再 import 副本。 | malformed/hash-mismatched archive 被原子拒绝，不出现 Dataset。 | Archive SHA-256/size、ZIP listing 或 changed hash、精确 error 与 Dataset list state。 |
| 在 Backtest 选择与 Dataset 不同的 Snapshot/Instrument/interval 并尝试绑定；再尝试把 Expected Value 绑定到 Probability slot。 | Snapshot mismatch 和 incompatible Strategy binding 都在 execution 前失败；不提供 approximate join、resampling、forward-fill 或 mixed Snapshot。 | Snapshot/Dataset/slot identities、candidate list 与精确 gate/error。 |
| 启动新的 native Dataset Attempt，在 Running 时选择 **Cancel**，再选择 **Retry**。另运行一个返回 non-finite output 的 disposable Model。 | Cancelled 与 Failed Attempts 保留 configuration、progress 和有界 diagnostics，不发布 partial Dataset；Retry 创建新 Attempt。 | 两个 Attempt IDs、terminal states、diagnostics、前后 Dataset list 与 technical errors。 |
| 检查标记为 Warmup/MissingInput 的 Dataset row，或使用 `availableAt` 晚于 decision time 的 external Signal，再运行兼容 Strategy。 | Run 记录 `Run Pause::MissingInput`；不替换为 zero、flat exposure、shifted row 或 future evidence。 | Row identity/status/availableAt、decision time、Run ID、pause evidence 与 Dataset lock。 |
| 在 `src-tauri` 运行 `cargo test datasets_lock_their_component_artifacts --lib`；然后在 **Settings → Data & Storage** 阅读相关 reset confirmation，但不确认。 | focused lock check 证明 referenced Dataset/Artifact deletion 被拒绝；reset copy 明确删除/保留范围并要求显式确认，同时保留账户/登录和设备偏好。 | Test output、引用 IDs、lock/reset message、summary counts 与技术详情；验收中不要执行破坏性 reset。 |

<!-- m8-acceptance:regressions -->
## 8. 桌面回归检查

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 将应用窗口设为 1024 px 宽，用键盘依次访问 Dashboard、market-data views、Components、Models 三个 tabs、Backtest、Validation 和全部 Settings sections。 | 内容可用且操作不被裁切；focus 可见；tabs、controls、tables/cards、pagination、status、warnings 与 errors 均可键盘操作，并使用文字/图标而非仅颜色表达含义。 | OS/display scale、page/tab、截图、focused element 与 accessibility text。 |
| 使用 titlebar Back/Forward 穿过 Models tabs 与 Backtest/Validation；访问其他页面后再返回。 | Route history 与 tab restoration 返回预期业务 page/tab，不显示 initialization。 | 精确 navigation sequence、expected/actual route/tab、截图和 console/technical error。 |
| Sign out，换另一个测试 User 登录，再访问 Components、Models、Backtest、Validation、Settings summary 和 market-data views。 | 用户范围内的 Components、Attempts、Datasets、Reports、Runs 与 Snapshot access 不泄漏；只有第二个 User 合法可访问的 evidence 才保持相同 immutable IDs。 | 两个经过隐藏处理的 User IDs、page、leaked/missing record ID、截图和技术详情。 |

<!-- m8-acceptance:automated-gates -->
## 9. 自动 release gates 与 CI

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 `src-tauri` 运行 `cargo test --workspace`、`cargo check --workspace` 和 `cargo fmt --all --check`。 | 全部 Rust unit、integration、doc、type 与 format checks 通过。 | Revision、失败命令、完整未过滤 failure、test name 和可用时的 backtrace。 |
| 在仓库根目录运行 `pnpm exec jest --watchman=false --runInBand`、`pnpm run build`、`pnpm run lint` 和 `git diff --check`。 | 全部 Jest suites、strict TypeScript/Vite build、scoped lint gate 与 whitespace check 通过；明确记录既有 lint warnings。 | Revision、命令、完整 error、suite/file 与 warning delta。 |
| 在 `examples/external-models/kronos` 运行 `python -m unittest test_adapter.py`；从干净 `dist/` 再次 build/verify 四个生成项目。 | Adapter fixture 与生成的 Model/Strategy package regressions 通过。 | Environment versions、完整输出、package path/hash 与 changed artifact listing。 |
| 推送 acceptance commit 后，记录所有适用 GitHub Actions run URL、commit SHA、platform/job 和 conclusion。 | reviewed revision 的必需 multi-platform checks 全部成功；local pass 不能替代 CI。 | Run URL/SHA、failed job/platform 和相关未删改 log excerpt。 |

<!-- m8-acceptance:acceptance-record -->
## 10. 验收记录

记录：macOS version/architecture 与 display scale；AdaQ revision；Rust/CLI/Node/pnpm/Python versions；四个 package hashes；只在私有记录中保存 User ID；native/external Dataset 与 Parquet hashes；Snapshot、Attempt、Artifact、Producer Segment、Feature Plan、Report、Run、Protocol 和 Validation Report IDs；三种 evaluation states；JSON/Markdown 与 `.adaq-signals` export names/hashes；negative-path evidence；accessibility/1024px review；CI URLs/conclusions。隐藏 credentials、OTPs、tokens、Supabase values、private paths 与 private market data。

维护者与 agent 每次只复核一个操作。只有上面每一行都通过，或可选的真实 Kronos 运行拥有完整的 unavailable evidence，全部自动 gates 和适用 CI 为绿色，且复核记录没有未解决 failure，才能接受 M8。

## 已交付范围边界

M8 交付 offline Single-Instrument inference、不可变 native/external Forecast Signal evidence、Forecast Evaluation、Dataset-first Signal-driven/Hybrid Backtests 与既有 Composed path。它不交付 training/fitting/tuning、embedded Qlib/Python、Cross-sectional inference、把 generated future paths 当成 realized data、live trading、Portfolio Optimization、OMS/EMS、controlled GPU/ONNX Runner、Marketplace publishing 或 future-profitability claims。
