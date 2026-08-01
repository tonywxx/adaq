# M8 人工验收（macOS ARM64）

这是 M8 的标准人工复核路径。每次只执行表中一行，失败时保留指定证据。Forecast Evaluation 衡量预测证据；Backtest 和 Validation 衡量 Strategy 行为。它们都不代表盈利承诺、Live Trading、Verified external inference 或 Marketplace 审批。

<!-- m8-acceptance:prerequisites -->
## 1. 前提条件、登录与存储

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在仓库根目录运行 `node --version`。 | 选中与 release CI 一致的 Node.js 24.x。 | 完整输出和 Node 安装方式。 |
| 运行 `corepack prepare pnpm@11.18.0 --activate`。 | 启用仓库固定的 pnpm 版本。 | 完整输出和 `corepack --version`。 |
| 运行 `pnpm install --frozen-lockfile`。 | 依赖与 `pnpm-lock.yaml` 一致。 | 完整输出、`node --version` 和 `pnpm --version`。 |
| 运行 `rustup toolchain install stable`。 | 安装 stable Rust 工具链。 | 完整输出和 `rustup show`。 |
| 运行 `rustup target add --toolchain stable wasm32-unknown-unknown`。 | 为 stable 安装 Component build target。 | 完整输出和 `rustup target list --installed --toolchain stable`。 |
| 运行 `cargo install cargo-component --locked`。 | `cargo component --version` 成功。 | 完整输出和 `rustc --version --verbose`。 |
| 运行 `cargo install --force --path src-tauri/crates/adaq-component-tooling`。 | `adaq-component --help` 列出 `new`、`build` 和 `verify`。 | 完整输出和 `cargo --version`。 |
| 在版本控制之外配置 `VITE_SUPABASE_URL` 与 `VITE_SUPABASE_PUBLISHABLE_KEY`，然后运行 `pnpm tauri dev`。 | 桌面登录界面出现，不显示缺少配置提示。 | 原始提示及变量名；绝不记录 value、密码、OTP 或 token。 |
| 用现有测试账户的 email 和 password 登录。 | Dashboard、sidebar 和当前 User 的研究数据出现。 | 可见错误及展开的技术详情，并隐藏秘密。 |
| 打开 **Settings → Data & Storage**，阅读摘要，再选择 **Open Data Folder**，不要执行 reset。 | 当前本地数据数量可读，应用数据目录打开；账户/登录与设备偏好不属于研究数据 reset 范围。 | 截图、可见/技术错误和 OS 版本。 |

Windows 使用 PowerShell、`py -3.12 -m venv .venv`、`.\.venv\Scripts\Activate.ps1`、`cargo install --force --path .\src-tauri\crates\adaq-component-tooling`、反引号续写 Adapter 命令、`C:\path\to\adaq.db`、`$env:TEMP\kronos-small.adaq-signals`，并用 `Get-FileHash -Algorithm SHA256 <path>` 代替 `shasum -a 256`。Linux 使用 `python3.12 -m venv .venv`、`. .venv/bin/activate`、`/path/to/adaq.db`、`/tmp/kronos-small.adaq-signals`、反斜杠续行和 `sha256sum`；macOS 使用 `shasum -a 256`。原生文件选择器与数据目录因平台而异。下面的标准人工复核运行以 macOS ARM64 为准。

<!-- m8-acceptance:components -->
## 2. 编写、构建、验证、导入与检查 Components

在新的空目录创建项目；已提交示例只作为参考，不能替代生成步骤。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 运行 `adaq-component new model m8-forecast-model`。 | 创建带生成 UUID 和 Model SDK feature 的空 Model project。 | 命令输出及生成的 `Cargo.toml`/`manifest.json`；不得重新生成或缩短 ID。 |
| 运行 `adaq-component new strategy m8-signal-strategy`。 | 创建带独立 UUID 的默认 Signal-driven Strategy project。 | 命令输出和生成的 `Cargo.toml`/`manifest.json`。 |
| 运行 `adaq-component new strategy m8-hybrid-strategy`。 | 创建带不同 UUID 的第二个默认 Strategy project。 | 命令输出和生成的 `Cargo.toml`/`manifest.json`。 |
| 运行 `adaq-component new strategy m8-composed-strategy --template composed`。 | 在 `kind: strategy` 下创建 Composed Strategy project。 | 命令输出和生成的 `Cargo.toml`/`manifest.json`。 |
| 用 [`model-close-score`](../examples/components/model-close-score/src/lib.rs) 的 identity-preserving 实现替换 `m8-forecast-model/src/lib.rs`，使用 `let normalized = (row.values.first().copied().unwrap_or_default() / 100.0).sin();` 和 `values: vec![normalized / 100.0, (normalized + 1.0) / 2.0, (normalized + 1.0) / 2.0]`。 | Batch Model 保留 Instrument ID、Prediction Time 与行顺序，并生成三个有变化、有限且确定的值。 | 完整 source 和 compiler error。 |
| 只编辑 `m8-forecast-model/manifest.json`：保留生成 identity fields；声明 `expected-return` 为 Expected Value/native `future-close-return`、`up-probability` 为 Probability/probability `future-close-up`、`return-score` 为 Score/percentile `future-close-return`，horizon 均为 1；增加均为 `0..0` 的 Artifact provenance windows `trainingWindow`、`fittingWindow`、`normalizationWindow`。 | Manifest 具有一个 market `close` Slot、Single-Instrument scope、三个有效唯一 outputs、embedded Artifact evidence 与 `warmupBars: 0`。 | 完整 Manifest 与 schema error；对照 [`model-close-score/manifest.json`](../examples/components/model-close-score/manifest.json)。 |
| 检查 `m8-signal-strategy`，不要编辑。 | 唯一 Forecast Signal Slot 使权威输入图为 Signal-driven。 | Source/Manifest 和意外字段。 |
| 只编辑 `m8-hybrid-strategy`：增加 market `close` Slot，绑定两个 indexes，并仅在 probability 至少为 `0.5` 且 close 为正时输出 `1`。 | Signal 加 Market Slots 使权威输入图为 Hybrid，且不存在 Architecture 字段。 | Source/Manifest 和 compiler error。 |
| 检查 `m8-composed-strategy`，不要编辑。 | 生成的 Market-only Slots 使权威输入图为 Composed。 | Source/Manifest 和意外字段。 |

对每个 project 分别执行下面两行，代入它的精确 directory 与 package name。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在一个生成 project directory 运行 `adaq-component build`。 | Tests、Wasm build、conformance 与 `dist/` 中的 self-contained package 成功。 | Project name、完整输出、package path/size 和 Manifest。 |
| 对同一 project 运行 `adaq-component verify dist/<project-name>-0.1.0.adaq`。 | Package integrity 与 contract verification 成功；记录 SHA-256。 | Project name、完整输出、package path 和 SHA-256。 |
| 在 **Components** 选择 **Import Component Package** 并只选中一个已验证 package。 | 该 package 被导入，精确 identity 与 compatibility evidence 可读。 | 所选文件、可见错误、展开详情和 package hash。 |
| 选择一次刚导入的 Component。 | IDs、versions、hashes、Model contract/Artifact 或 Strategy Slots 及推导出的 Architecture 可读、可复制。 | 截图、精确 IDs/hashes 和失败 detail。 |

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
| 在 `src-tauri` 运行 `cargo test kronos_fixture_reaches_import_evaluation_and_dataset_first_backtest --lib`。 | committed deterministic fixture 被 import、按 Unknown evaluation、绑定 compatible Strategy、执行 Dataset-first Backtest，并保留在 Run Dataset Lock 中。 | 完整输出、失败 stage/test、fixture archive SHA-256 与可用时的 backtrace。 |
| 按[外部 Kronos Adapter](../examples/external-models/kronos/README.zh-CN.md)取得固定 revision 的 `Kronos-small`，记录 licence 与 weight/config hashes。 | 精确 inference Model Artifact 可检查，且未公开 credentials。 | Command、revision/URL、licence、HTTP error、size 与 hashes。 |
| 取得固定 revision 的 `Kronos-Tokenizer-base`，记录 licence 与 weight/config hashes。 | 精确 Tokenizer Artifact 可用，且不被描述为 inference model。 | Command、revision/URL、licence、HTTP error、size 与 hashes。 |
| 用第 3 节精确 Snapshot、Seed `7` 和明确选择的 CPU/MPS/CUDA device 运行 Adapter 一次。 | 生成与 Snapshot 对齐的 `kronos-small.adaq-signals`。如果硬件/网络阻止这项可选真实权重操作，保留指南要求的完整 unavailable evidence，不声称已经运行。 | Runtime/config、Seed、device、peak memory、elapsed time、完整 traceback，且不含 credentials/private data。 |
| 在 **Models → Signal Datasets** 选择 **Import .adaq-signals** 并选中生成的 archive，检查后用 **Export .adaq-signals** 保存到新路径。 | Import 完整验证并原子发布精确 external Dataset；证据保持 **Externally Generated**，保留 Producer Segment、Artifact/weight/Tokenizer/Adapter/preprocessing hashes、unknown training evidence、Snapshot alignment、availability policy 与相同 authoritative export identity。 | Archive SHA-256/size、检查过的 Manifest、精确 typed error、是否出现 Dataset、Dataset ID 和 export path/error。 |

<!-- m8-acceptance:evaluation -->
## 5. Expected Value、Probability 与 Score 评估

对原生 Dataset 的 `expected-return`、`up-probability` 和 `return-score` 分别重复下列操作。使用 Dataset coverage 边界，stability window 设为 `20`。

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 **Models → Evaluation Reports** 选择 Dataset 与 Signal，输入完整起止毫秒和 `20`，再选择 **Create Report**。 | 不可变 Report 显示共同的 coverage/missingness/distribution/stability；Expected Value 显示 MAE/RMSE/bias/correlation，Probability 显示 Brier/Log Loss/ROC AUC/calibration，Score 显示 time-series Pearson IC/Spearman IC/window IC/ICIR/quantiles。 | Dataset/Signal/horizon/window、typed error、unavailable rows，以及已创建时的 Report ID。 |
| 对每个渲染出的 metric，用 Tab 聚焦相邻 information control，以键盘打开并关闭，然后分别用 click 和 pointer hover 打开。 | 无需只依赖颜色或 hover 即可读取 meaning、formula、interpretation direction、range、caveat、undefined state 和 reference link；预测质量不会被表述成 Strategy profitability。 | Metric label、interaction mode、截图、focus state 和 accessibility-tree text。 |
| 展开每个 Report 的 **Evidence** 与 **Provenance**，再用新文件名选择 **Export JSON** 和 **Export Markdown**。 | Producer-level evidence 与 unavailable results 保持可见；精确 Dataset/Snapshot/Segments/Artifacts/contracts/hashes/trust/versions 被保留；不会覆盖已有文件。 | Report ID、Evidence State、export name、可见/技术错误，以及安全时的 exported file。 |
| 复制一次 Model project，保留 `componentId`，把 version 改为 `0.1.1`，并把三个 provenance windows 改为 `0..9999999999999`。 | 新 project 表示同一 Component 的下一版本，带故意重叠的 evidence。 | 两份 Manifests 与精确 changed fields。 |
| 在 `0.1.1` project 运行 `adaq-component build`。 | Overlapping-evidence package build 并通过 conformance。 | 完整输出和 Manifest。 |
| 运行 `adaq-component verify dist/m8-forecast-model-0.1.1.adaq`。 | Package verify 成功并具有新 archive hash。 | 完整输出、path 与 archive hash。 |
| 在 **Components** 只 import `m8-forecast-model-0.1.1.adaq`。 | Version `0.1.1` 出现在保留的 Component identity 下。 | 所选文件、可见/技术错误和 package hash。 |
| 在 **Models → Create Dataset** 用 version `0.1.1` 与同一 Snapshot 创建一个 Dataset。 | 带 overlapping provenance 的不同 immutable Dataset 完成。 | Attempt/Dataset IDs、status、diagnostics 与 hashes。 |
| 从 `0.1.1` Dataset 创建一个 Report。 | Evidence State 为 **Overlapping**，原始 native Reports 仍为 **Out-of-sample**。 | Dataset/Report IDs、Segment windows、states、warning 与 provenance JSON。 |
| 从 imported Kronos Dataset 创建一个 Report。 | 不完整 upstream windows 产生 **Unknown**，且不升级 trust。 | Dataset/Report ID、state、warning 与 provenance JSON。 |

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
| 在 disposable Model Manifest 把 `horizonBars` 设为 `0`，再运行 `adaq-component build`。 | Invalid horizon 在 import 前以稳定 typed evidence 失败。 | Mutated Manifest、完整输出和 `dist/` 是否变化。 |
| 恢复 horizon，把 Probability 改为 `future-close-return`，再运行 `adaq-component build`。 | Invalid Kind/Target combination 在 import 前失败。 | Mutated Manifest 和完整 typed error。 |
| 在有效 `.adaq-signals` 的副本中加入一个意外 ZIP entry。 | 生成 malformed disposable archive，且未编辑 finalized evidence。 | Source/copy paths、ZIP listing、size 与 SHA-256。 |
| 只 import 上述 malformed archive。 | 它被原子拒绝，不出现 Dataset。 | 精确 error 与前后 Dataset list。 |
| 修改第二个 disposable archive 内的 `signals.parquet` 并 import。 | Hash mismatch 被原子拒绝。 | Archive SHA-256/size、changed Parquet hash、精确 error 与 Dataset list state。 |
| 在 Backtest 选择与 Dataset 不同的 Snapshot/Instrument/interval，并检查 compatible Signal candidates。 | Snapshot mismatch 阻止绑定；不提供 approximate join、resampling、forward-fill 或 mixed Snapshot。 | Snapshot/Dataset identities、candidate list 与精确 gate。 |
| 尝试把 Expected Value Signal 绑定到 Probability slot。 | Incompatible Strategy binding 不出现或在 execution 前被拒绝。 | Slot/Signal contracts、candidate list 与精确 error。 |
| 启动一个新 native Dataset Attempt，在 Running 时选择 **Cancel**。 | Cancelled Attempt 保留 configuration、progress 与 diagnostics，且不发布 partial Dataset。 | Attempt ID、terminal state、diagnostics 与前后 Dataset list。 |
| 在上述 Cancelled Attempt 选择 **Retry**。 | Retry 创建新的 Attempt identity。 | 旧/新 Attempt IDs、status 与技术错误。 |
| 运行一个返回 non-finite output 的 disposable Model。 | Failed Attempt 保留有界 diagnostics，且不发布 partial Dataset。 | Attempt ID、diagnostic、Dataset list 与精确 technical error。 |
| 跨一个已检查的 Warmup/MissingInput row，或 `availableAt` 晚于 decision time 的 external Signal，运行 compatible Strategy。 | Run 记录 `Run Pause::MissingInput`；不替换为 zero、flat exposure、shifted row 或 future evidence。 | Row identity/status/availableAt、decision time、Run ID、pause evidence 与 Dataset lock。 |
| 在 `src-tauri` 运行 `cargo test datasets_lock_their_component_artifacts --lib`。 | Focused check 证明 referenced Dataset/Artifact deletion 被拒绝。 | 完整 test output、引用 IDs 和可用时的 backtrace。 |
| 在 **Settings → Data & Storage** 打开相关 reset confirmation，然后选择 **Cancel**。 | Copy 明确删除/保留范围并要求显式确认，同时保留账户/登录与设备偏好。 | Reset message、summary counts 与技术详情；验收中不要执行破坏性 reset。 |

<!-- m8-acceptance:regressions -->
## 8. 桌面回归检查

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 将应用窗口设为 1024 px 宽，用键盘依次访问 Dashboard、market-data views、Components、Models 三个 tabs、Backtest、Validation 和全部 Settings sections。 | 内容可用且操作不被裁切；focus 可见；tabs、controls、tables/cards、pagination、status、warnings 与 errors 均可键盘操作，并使用文字/图标而非仅颜色表达含义。 | OS/display scale、page/tab、截图、focused element 与 accessibility text。 |
| 使用 titlebar Back/Forward 穿过 Models tabs 与 Backtest/Validation；访问其他页面后再返回。 | Route history 与 tab restoration 返回预期业务 page/tab，不显示 initialization。 | 精确 navigation sequence、expected/actual route/tab、截图和 console/technical error。 |
| Sign out，换另一个测试 User 登录，再访问 Components、Models、Backtest、Validation、Settings summary 和 market-data views。 | 用户范围内的 Components、Attempts、Datasets、Reports、Runs 与 Snapshot access 不泄漏；只有第二个 User 合法可访问的 evidence 才保持相同 immutable IDs。 | 两个经过隐藏处理的 User IDs、page、leaked/missing record ID、截图和技术详情。 |
| 检查 root Documentation table 列出的 README、SDK、Component、archive、Metric、external-model 和两份 manual-acceptance 文档。 | Delivered M8 scope 一致，且都不声称 training、embedded Qlib/Python、Cross-sectional inference、live trading、Portfolio Optimization、OMS/EMS、Marketplace publishing 或 future profitability。 | File/link、冲突原文与期望 scope statement。 |

<!-- m8-acceptance:automated-gates -->
## 9. 自动 release gates 与 CI

| 精确操作 | 预期结果 | 失败时保留 |
| --- | --- | --- |
| 在 `src-tauri` 运行 `cargo test --workspace`。 | 全部 Rust unit、integration 与 doc tests 通过。 | Revision、完整未过滤 failure、test name 与可用时的 backtrace。 |
| 在 `src-tauri` 运行 `cargo check --workspace`。 | 全部 Rust workspace type checks 通过。 | Revision 与完整输出。 |
| 在 `src-tauri` 运行 `cargo fmt --all --check`。 | 全部 Rust files 符合 rustfmt。 | Revision 与完整 diff。 |
| 在仓库根目录运行 `pnpm exec jest --watchman=false --runInBand`。 | 全部 Jest suites 通过。 | Revision、suite/test 与完整 error。 |
| 运行 `pnpm run build`。 | Strict TypeScript 与 production Vite build 通过。 | Revision 与完整 error。 |
| 运行 `pnpm run lint`。 | Lint 命令成功；记录明确的既有 warnings。 | Revision、file/rule、完整输出与 warning delta。 |
| 运行 `git diff --check`。 | 不存在 whitespace errors。 | Revision 与完整输出。 |
| 在 `examples/external-models/kronos` 运行 `python -m unittest test_adapter.py`。 | Pinned environment 中两个 Adapter fixture tests 通过。 | Environment versions 与完整输出。 |
| 只删除每个生成 project 的 `dist/` 后，在各 project 分别重新运行一次 `adaq-component build`。 | 四个 generated-project regressions 全部通过。 | Project name、完整输出与 artifact listing。 |
| 对每个 rebuild project 分别重新运行一次 `adaq-component verify dist/<project-name>-0.1.0.adaq`。 | 四个精确 packages 全部 verify；hashes 与 acceptance record 一致。 | Project name、完整输出、path 与 hash。 |
| 推送 acceptance commit 后，记录所有适用 GitHub Actions run URL、commit SHA、platform/job 和 conclusion。 | reviewed revision 的必需 multi-platform checks 全部成功；local pass 不能替代 CI。 | Run URL/SHA、failed job/platform 和相关未删改 log excerpt。 |

<!-- m8-acceptance:acceptance-record -->
## 10. 验收记录

记录：macOS version/architecture 与 display scale；AdaQ revision；Rust/CLI/Node/pnpm/Python versions；四个 package hashes；只在私有记录中保存 User ID；native/external Dataset 与 Parquet hashes；Snapshot、Attempt、Artifact、Producer Segment、Feature Plan、Report、Run、Protocol 和 Validation Report IDs；三种 evaluation states；JSON/Markdown 与 `.adaq-signals` export names/hashes；negative-path evidence；accessibility/1024px review；CI URLs/conclusions。隐藏 credentials、OTPs、tokens、Supabase values、private paths 与 private market data。

维护者与 agent 每次只复核一个操作。只有上面每一行都通过，或可选的真实 Kronos 运行拥有完整的 unavailable evidence，全部自动 gates 和适用 CI 为绿色，且复核记录没有未解决 failure，才能接受 M8。

## 已交付范围边界

M8 交付 offline Single-Instrument inference、不可变 native/external Forecast Signal evidence、Forecast Evaluation、Dataset-first Signal-driven/Hybrid Backtests 与既有 Composed path。它不交付 training/fitting/tuning、embedded Qlib/Python、Cross-sectional inference、把 generated future paths 当成 realized data、live trading、Portfolio Optimization、OMS/EMS、controlled GPU/ONNX Runner、Marketplace publishing 或 future-profitability claims。
