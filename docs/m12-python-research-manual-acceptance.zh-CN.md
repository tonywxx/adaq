# M12 Python Research 人工验收指南

[English](./m12-python-research-manual-acceptance.md)

状态：这是已接受的 2026-08-13 Q93 合同的实现验收矩阵，适用于 [M12 Python Research SDK 与 Qlib-first Model Lab 架构](./m12-python-research-and-model-lab.zh-CN.md)。Parent Specification 为 [#97](https://github.com/tonywxx/adaq/issues/97)；本地检查只证明其直接覆盖的条目，支持平台验收仍需单独记录。

本指南分开三个 Gate：

- M12 验收共享 Python 基础、Python Factor 与 Qlib Ridge Model 路径。
- M13 在同一合同上加入 Python Strategy 与完整 Tutorial Backtest。
- M14 在同一合同上加入生成的 WASI Component 与 Equivalence。

不得在较早 Milestone 中把延期项目标记为通过。

## 1. 验收记录

测试前记录：

| 字段 | 必需证据 |
| --- | --- |
| 被审查 Revision | 完整 Git SHA，不包含未审查的生成文件或本地源码变更 |
| Application | ADAQ Version 与 Build Identity |
| Platform | macOS ARM64、Windows x86_64、Linux x86_64 |
| Toolchain | Rust、Node、pnpm、Managed CPython、SDK、Runner、Qlib Adapter Version |
| Runtime Artifact | Platform、Version、Source、Signature/Hash 与 Installed Bytes |
| Wheelhouse | Manifest/Signature Hash 与每个选定 Wheel Hash |
| Fixture | `python-tutorial-a-share@1` Manifest 与 Content Hash |
| Example Revision | 所有适用 `py-*` Project 的精确 Hash |
| Test User | 已脱敏 User Identity 与干净或保留的 Local-data State |
| CI | 每个平台的 Workflow URL、Revision、Job URL 与 Conclusion |
| Manual Environment | OS Version、Architecture、Display Scale、Locale 与所用 Assistive Technology |

验收记录不得包含 Credential、Provider Token、Signing Key、私有绝对路径或无界 Python Output。

## 2. 前置条件与边界

执行前确认：

- M11 已完成，并且测试者理解精确的 Factor Research Schema Policy。
- 未安装 ADAQ Python Runtime 时，App 可以启动，非 Python Route 正常工作。
- 测试不使用 System Python、Conda Environment、Active Virtualenv 或 User `PATH` Interpreter。
- 三个 Example 位于 `examples/python/`；共享 Dataset Fixture 位于 `src-tauri/fixtures/python-tutorial/`，且不在任何 Project Archive 中。
- Fixture 清晰标明全部 12 个 Instrument 与所有 Price History 均为合成数据。
- 使用可丢弃的 User Profile，并有足够磁盘容纳一个 Runtime、Wheelhouse、Project Environment 与保留 Evidence。
- 只有显式 Runtime 或 Wheel Preparation 步骤允许 Network；Research Attempt 不依赖 Network Data。

负向边界检查不得发现 Embedded IDE、Jupyter Server、通用 Scripts Page、Python Order API、Child Environment 中的 Credential、Qlib Data Downloader/Provider、Alpha158 隐式 Input、通用 Qlib Model 承诺、通用 Python-to-WASM Converter 或 Marketplace Publication UI。

## 3. Project 与 Archive 合同

对每个适用 Example 执行：

| 操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| Create from Example | 创建一个带精确 Kind 前缀 ID 的 User Working Copy；不覆盖已有 Copy。 | Requested ID、Existing ID、Path-safe Diagnostic |
| 检查 Root | 只有接受的 Manifest、Project Metadata、Lock、`src/`、Docs 与 Licence 结构具有权威性。 | Unexpected 或 Missing Path |
| 无 Runtime 时 Validate | 不安装或 Import Python 即可完成 Static Validation。 | Validation Stage 与 Diagnostic |
| 编辑已声明 Source File | State 变为 Dirty；不加载 Module，也不启动 Attempt。 | 变更前后 Source Hash 与 Process List |
| 引入未知 Manifest Field 或 Enum | State 变为 Invalid；禁用 Prepare/Run；Source 仍可打开和导出。 | Schema Version、Field Path、Typed Code |
| 修改 Project ID 前缀或 Kind | Validation 拒绝不匹配；历史 Identity 不变。 | ID、Kind、Revision Hash |
| 恢复有效 Source 后 Run | 执行前冻结一个不可变 Revision；后续编辑不影响它。 | Frozen File List 与 Hash |

Archive Validation 必须覆盖：

- 相同 Revision 的确定性重新导出产生完全相同的 ZIP Bytes 与 Hash。
- 私有本地导出只有在 `LICENSE` 内容匹配时才接受 `LicenseRef-Proprietary`。
- 内置 Example 使用 Apache-2.0 导出，并包含两种语言的 Guide。
- 未来 Community Eligibility Check 即使 Price 为零，也会拒绝 Proprietary 或缺少再分发权的 Project。
- Import 只复制一个 Untrusted Working Copy，不执行 Runtime Preparation、Module Import 或代码。
- Absolute Path、`..`、Symbolic Link、Hard Link、Duplicate Path、Case-fold Collision、Undeclared Entry、Count/Size Overflow 与 Lock Hash Mismatch 均在复制前被拒绝。

## 4. Runtime、Lock 与 Environment

从未安装 ADAQ Python Runtime 的设备开始：

| 操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| 浏览非 Python ADAQ Workflow | 仍可使用，且不会提示安装 Python。 | Route 与意外 Prompt/Process |
| 打开 Python Project 并 Validate | 无需下载 Runtime 即可通过 Validation。 | Project 与 Validation Evidence |
| Prepare Environment | 显式准备前显示精确 CPython 3.12.x、Platform、Download Size、Disk Requirement、Source 与 Hash。 | Artifact Identity 与 UI State |
| 中断 Runtime Download 或 Unpack | 未完整 Staging Runtime 不得变为可执行；Retry 创建新的 Preparation Attempt。 | Attempt ID、脱敏 Staging Path、Hash |
| 完成 Preparation | Signature/Hash Verification 通过，Atomic Publication 生成一个精确 Runtime 与 Environment Identity。 | Runtime/Environment Hash |
| 发起两个相同 Preparation | Request 合并；不出现竞争 Setup 或 Research Process Pool。 | Attempt Linkage 与 Process Count |
| 编辑 `pyproject.toml` | Working Copy 变为 Dirty；Run 不解析或安装任何内容。 | Source/Lock Hash |
| Sync Environment | 只从 Trusted Index 解析 Wheel，并原子替换 `pylock.toml`；新 Revision 为 Untrusted。 | Selected Wheel 与 Hash |
| 插入 Source Distribution、Build Script、不支持的 Native Wheel 或错误 Hash | Sync/Prepare 在不运行 Package Code 的前提下拒绝。 | Package Identity 与 Typed Reason |
| 删除未使用 Environment Cache | 历史 Evidence 仍可读；Rerun 可重建精确且获准的 Bytes。 | 删除前后 Disk Use 与 Identity |
| 尝试安全禁用的旧 Runtime | Evidence 仍可检查，但 Execution 被阻止。 | Profile、Disabled Reason、No-process Proof |

Run 不得访问 Package Index、重写 `pylock.toml`、使用 System Site-packages 或选择更新的公开 SDK Wheel。

## 5. Trust、Runner、Protocol 与 Lifecycle

| 操作 | 预期结果 | 失败时记录 |
| --- | --- | --- |
| Import 或 Prepare Untrusted Project | 不授予 Execution Trust。 | Trust View 与 Revision Hash |
| 选择 Run | 确认前显示精确 Revision、Entry Point、Lock、Source List、Resource Policy 与 Trusted-code Warning。 | 缺少的 Disclosure |
| 拒绝 Trust | 不 Import Project Module，也不启动 Research Attempt。 | Process/Attempt Proof |
| 接受 Trust | 一个 Trust Decision 只绑定该精确 Revision。 | Decision 与 Revision Hash |
| 修改 Source、Entry Point 或 Lock | 只有该 Project 变为 Untrusted；其他 Tutorial Decision 仍有效。 | 变更前后 Revision 与 Decision |
| 启动 Attempt | Host 选择 Random Loopback Port 与 One-time Token；Project Import 前完成精确 Protocol/SDK/Revision/Attempt Handshake。 | 脱敏 Handshake Identity |
| 重用 Token、远程连接或 Handshake Field 不匹配 | Execution Fail-closed，不发布 Result。 | Typed Code 与 No-publication Proof |
| 输出 stdout/stderr | 只作为有界 User-scoped Log 显示，不成为 Protocol Data，也不自动上传。 | Log Cap 与 Redaction Evidence |
| Retry Terminal Attempt | 新 Attempt 引用 Source Attempt；旧 Evidence 不可变。 | Attempt ID |
| App 在 Pending/Running Work 时重启 | Pending 保持排队；过期 Running 变为 Failed/Interrupted；Late Result 不得修改它。 | 变更前后 State |
| Cancel Cooperative/Non-cooperative Project | Host 先请求取消，Grace 后终止 Process Tree；仅在退出并隔离 Staging 后，Cancelled 才为终态。 | Timing、Process-tree Proof、Staging State |

检查 Child Environment 与保留 Log，确认其中没有 Credential、Provider Token、Signing Key、Order Endpoint、SQLite Path、Internal Parquet Layout 或 Private Absolute Path。不得把 Process Isolation 描述为任意代码的强 Sandbox。

## 6. M12 Python Factor 路径

使用 `py-factor-cross-sectional-momentum` 与 `python-tutorial-a-share@1`。

1. 验证 Project 的 Kind 为 Factor、Mode 为 Portable Definition、Scope 为 Cross Sectional、Entry 为 `project:create_project`，Licence 为 Apache-2.0。
2. 确认 Definition Phase 不接收 Dataset，只使用现有 Feature Operator Catalog 构造 Graph：

   ```text
   close → backward-simple-return(lookback) → cross-sectional-percentile → momentum-score
   ```

3. 注册 Host-owned Grid `lookback={5,20,60}`。确认存在三个独立 Factor Trial 与 Attempt；不得接受隐藏的 Python Sweep Result。
4. 在既有 M11 Protocol 中绑定精确 Snapshot、Point-in-Time Universe、Feature Evidence、Window、Target、Seed 与 Engine Identity。
5. 通过 Python Candidate Source 物化每个标准 Factor Dataset。确认精确 Row Identity、确定性 Universe Order、Finite Binary64 或 Typed Unavailable Value，以及 Atomic Publication。
6. 在 Fresh Process 与允许的 Batch Partition 下重复运行。Repeatability Report 必须以精确 Output Equality 标记 Verified。
7. 运行既有 M11 Scope-correct Evaluation，检查 Family Lineage、每个 Trial、Selection/Final Window、Missingness、Metric 与 Evidence State。
8. 记录 User 对 Lookback 20 的 Parameter Selection Decision。UI 可以建议 Tutorial Default，但不得自动创建 Decision。
9. 检查 Evaluation Report，然后显式记录 Research Validated。没有该 User Promotion Decision 时，Model Step 保持阻塞。
10. 确认 Model Research 可选择绑定精确 Promoted Dataset Output、Report、Policy、Decision、Revision、Environment 与 Engine Provenance 的证据。

失败路径必须覆盖 Invalid/Custom Portable Operator、在 `define` 中访问 Dataset、Identity Reordering、Missing Member、Silent Row Deletion、NaN/Infinity、错误 Output Count、Exception、Cancellation、Divergent Replay 与不兼容 `FACTOR_RESEARCH_SCHEMA_VERSION`。Imperative Python 可以保留为可检查的 Research Validated Evidence，但没有受支持的 Portable Representation 时绝不能成为 Component Eligible。

## 7. M12 Qlib Ridge Model 路径

只有精确 Factor Evidence 获得正向 User Decision 后，才使用 `py-model-qlib-ridge-return`。

1. 验证一个 Model Kind、一个 Continuous Future Close Return Target、五 Bar Horizon、一个 Forecast Signal、Qlib Ridge Adapter Identity 与 Apache-2.0。
2. 通过有序 Input Slot 绑定相同 Synthetic Snapshot 以及精确 Promoted Factor/Feature Input。
3. 验证固定 Window：

   | 用途 | Session |
   | --- | --- |
   | Train | 1–100 |
   | Purge | 101–105 |
   | Selection Validation | 106–140 |
   | Embargo | 141–145 |
   | Final Evaluation | 146–180 |

4. 确认跨越边界的五 Session Target 为 Unavailable，而不是被移动。
5. 检查 `adaq.qlib` View：稳定 `(datetime, instrument)` Order；只包含 `train`、`valid` 与 Feature-only `test`；没有 Provider Initialization、Qlib Data Directory、Alpha158、Downloader 或 Network。
6. 确认 Host-owned Standardization 只在 Train 上 Fit，并冻结一个 Fitted Transformation Artifact，之后保持不变地应用。
7. 把 `alpha={0.1,1,10}` 注册为三个独立 Trial 与 Attempt。Host 使用仅在 Train 拟合的变换，基于 Train/Selection Validation Label 计算并记录 Selection MSE；用户输入的指标不具备权威性。
8. Final Evaluation 前记录一个 User Parameter Selection Decision。Test Label 永不进入 Python；Host 计算 Final Metric。
9. 提取 `adaq:linear-model@1`，然后在发布 Forecast 前重新加载。检查有序 Input Slot、Finite Coefficient、Intercept、Numeric Representation、Transformation Identity、Forecast Contract 与 Adapter Provenance。
10. 扫描 Project Archive、Artifact、Staging、Final Dataset 与 Component Input，不得存在 Pickle 或 Executable Object Graph。
11. 在现有 M8 Contract 下生成不可变 Forecast Signal Dataset 与 Forecast Evaluation Report。
12. 在 Fresh Process 中 Replay。Coefficient 与 Forecast 必须处于注册的严格有限 Tolerance 内；Identity、Availability、Order 与 Contract 保持精确。

不支持的 Qlib Model、多个 Target、隐式 Qlib Processor、Custom Preprocessing、任意 Serialization、通用 ONNX Export 或 Local Qlib Paper 声明必须保持 Research Only，或者被显式 Adapter Eligibility 拒绝。不能仅因可 Import 而继承 Ridge 支持。

## 8. 引导式 Tutorial 行为

Run Python Tutorial 必须：

1. 显示精确适用 Project Revision、Entry Point、Lock、Runtime/Wheel Download、Disk Need、Licence 与 Trusted-code Warning。
2. 允许一次确认记录彼此独立的 Trust Decision；不得创建 Blanket/Future Trust。
3. 机械地完成 Validate、Prepare 与 Navigation，同时 App 保持响应。
4. Factor Evaluation 后停止，直到 User 记录 Parameter Selection 与 Research Validated Promotion Decision。
5. Model Selection Evidence 后停止，直到 User 记录其 Parameter Selection Decision。
6. 对该 Decision 只运行一次 Held-out Final Evaluation；之后受反馈驱动的工作必须真实标记为 Overlapping。
7. M12 结束于可检查的 Factor/Model Evidence，并把 Strategy 标识为 M13 Continuation，而不是 Failure 或 Hidden Step。

每个显示的 Return 或 Ranking 都必须标为 Synthetic Demonstration，不得暗示预期盈利。中英顶层与各 Project Guide 必须描述相同 Button、Path、Parameter、Boundary、Expected Structure 与 Troubleshooting。

## 9. Result Validation、Failure 与 Recovery

下表每项都必须保留 Typed Bounded Diagnostic，且不得发布可消费的 Partial Result：

| Failure | 预期终态行为 |
| --- | --- |
| Invalid Manifest、Unsupported Schema、Entry-point Mismatch | Preparation 或 Project Import 前标记 Invalid |
| Archive Traversal、Link、Collision、Undeclared/Oversized Entry | 复制前拒绝 Import |
| Runtime/Wheel Hash 或 Signature Mismatch | Preparation Failed；不存在可执行的 Partial Environment |
| Untrusted Revision | Import 前阻止 Run |
| Handshake/Token Mismatch | Project Code 前 Attempt Failed |
| Oversized Control、Arrow、Artifact、Checkpoint 或 Log | Typed Limit Failure；需要时停止 Process |
| Duplicate/Reordered Identity、Invalid Dtype/Schema、NaN/Infinity、Invalid Decimal | Host Validation Failure；隔离 Staging |
| 缺少 Required Factor/Model Input | 按合同产生 Typed Unavailable 或 Failed Input Gate；不得静默 Fill/Drop |
| Python Exception 或 Child Crash | Failed，保留有界 Traceback，无权威 Partial Result |
| Cancel 或 App Restart | 只有隔离后才进入 Cancelled 或 Failed/Interrupted；Retry 是新 Attempt |
| Cancel/Restart 后的 Late Result | 忽略；历史 Attempt 不变 |
| Repeatability Divergence | Result 可检查；阻止 Promotion、Generation 与 Qualification |
| Test-label Access 或 Overlapping Selection | 拒绝访问或标记 Evidence State 为 Overlapping；绝不标为 Out-of-sample |
| 尝试写 SQLite/Final Dataset | 不提供 Path 或 Authority；Host 保持唯一 Publisher |

Resource Test 必须用经过测量的 Platform Policy 覆盖 Wall Time、Memory、Thread、Input Row/Column/Cell、Message、Artifact、Checkpoint、Log 与 Process-count Cap。Project Request 可以降低但绝不能提高 Host Limit。

## 10. Lab、Settings、Localization 与 Accessibility

对 M12 的 Factor/Model Lab：

| 操作 | 预期结果 |
| --- | --- |
| 直接导航到 Lab | Route Shell 立即 Paint；不以等待 Python 或 Evidence 的全局 Blocking Loader 阻塞。 |
| Create/Open Python Project | Project 出现在所属 Lab，而不是通用 Scripts Page。 |
| Prepare、Run、Cancel、Sync、Export | Pending/Error/Progress State 属于发起操作的 Control 或 Project/Attempt Row。 |
| Inspect Project | Clean/Dirty/Invalid、Missing/Preparing/Ready/Failed、Untrusted/Trusted、Latest Attempt、Hash 与 Evidence Link 可见。 |
| Open Project Folder | 在外部 Editor/Folder 中打开，不提供 Embedded Editor、Notebook 或 Terminal。 |
| Inspect Settings | Runtime Profile、Wheelhouse/Environment Disk Use 与显式 Inactive-cache Removal 可见；不存在 Custom Interpreter Picker。 |
| 使用 Keyboard 与 Screen Reader | Action、Warning、Progress、Log、Table、Dialog、Decision Boundary、Focus Restoration 与 Status Announcement 均可使用。 |
| 切换 en-US/zh-CN | UI 与 Docs 立即本地化；ID、Hash、Schema Code、Decimal String 与 Evidence Identity 不变。 |

测试 Slow Operation、Rapid Navigation、Cancellation、Stale Response、Narrow Window、200% Scale 与每个支持 OS。Trust 与破坏性 Reset Dialog 必须清楚说明 Scope，并正确处理 Focus。

## 11. Schema Reset 与 Cache Retention

创建受控的不兼容 Metadata 并验证：

- `PYTHON_RESEARCH_SCHEMA_VERSION` 从 `1.0.0` 开始。
- 不兼容 Python Metadata 阻止 Python Research，并明确引导 Device-level Reset Python Research Evidence。
- Reset 停止 Python Research，删除 Revision、Attempt、Trust、Binding 与 Result Metadata。
- User Working Copy 与 Exported Project Archive 保持不变。
- Runtime、Wheelhouse 与 Environment Cache 可单独删除，不与 Evidence Reset 混淆。
- 不兼容 Factor Candidate Evidence 使用独立且已接受的 Factor Research Reset，把 `1.0.0` 升至 `1.1.0`。
- 两种 Reset 都不执行 Migration、Dual-read、Silent Deletion 或完整 Local Data Reset。

记录 Reset 前后 Count 与 Hash，并证明其他 User 的 Working Copy Source 未被删除。

## 12. M13 Strategy 扩展 Gate

以下项目在 M12 延期，并在 M13 成为强制要求：

- `py-strategy-top-n-forecast` 独立有效、Apache-2.0、Offline，并复制到 Strategy Lab。
- `start(context)` 创建一个 Segment-local Session；有序 `decide` Call 串行执行，绝不预取 Future Batch。
- 缺少 Required Input 时，在调用前记录 `Run Pause::MissingInput`。
- 有限 Grid 为 `forecast-weight={0.5,0.7}`、`top-n={3,5}`、`cash-reserve={0,0.1}`，默认值分别为 0.7、3、0.1。
- Portable Operation 只包含 Weighted Sum、Deterministic Top-N、Equal Weight 与 Cash Reserve。
- 分数相同时按 Instrument ID 升序。
- Long-only Target 包含每个 Universe Member；Nonnegative Decimal Weight 与 Cash Reserve 精确等于一。
- Host Risk、Execution、Backtest 与 Portfolio State 保持权威；Python 不输出 Order。
- Strategy Repeatability 与 Golden Portfolio Target 必须精确。
- Tutorial 在 Held-out Backtest 前要求显式 Strategy Parameter Selection Decision。
- Short、Leverage、Custom Eligibility、Optimizer、Stop、Loop、Callback、Order 与 Qlib-native Backtest Promotion 仍排除在外。

## 13. M14 Generation 扩展 Gate

以下项目在 M12/M13 延期，并在 M14 成为强制要求：

- Portable Factor/Strategy Definition 与 `adaq:linear-model@1` 是三个 Tutorial 唯一的 Generation Input。
- 固定 Rust SDK Generator 接收规范 Definition 或 Artifact Data，绝不执行 Python。
- `.adaq` 不包含 Python Source、Runtime、Wheel、Environment、Lock、Dataset 或 Research Result。
- 选定 Factor/Strategy Value 成为 Default；每个有限 Allowed Combination 都在 Host Cap 内通过 Conformance 与 Equivalence。
- Ridge Model Exporter 生成一个 WASI Model Component；通用 Qlib-to-WASM/ONNX 与 Local Qlib Paper 仍不支持。
- Generated Component Provenance 绑定 Revision、Definition/Artifact、Parameter Schema、Decision、Generator、SDK、ABI、Toolchain、Build Attempt 与 Equivalence Report。
- 进入 Component Library 前，Build、Conformance、Numeric-boundary、Resource、Provenance、Equivalence、Trust、Package Validation、Identity 与 Import Gate 必须全部通过。
- Failure 保留 Evidence，绝不覆盖已有 Package Identity/Version。
- 精确 Factor/Strategy Behavior 与 Tolerance-governed Ridge Forecast Equivalence 在所有支持平台通过。

## 14. 自动化 Gate

M12.1 必须新增并记录 Repository-managed Python Package/Contract Test Entry Point；验收证据不得用开发者的 System Python Invocation 代替这个未来命令。在被验收 Revision，记录精确已提交命令，且至少运行：

```sh
(
  cd src-tauri
  cargo fmt --all --check
  cargo test -p adaq-python-research
  cargo test --workspace
  cargo check --workspace
)
pnpm exec jest --watchman=false --runInBand
pnpm run build
pnpm run lint
git diff --check
```

还要运行各 Slice 新增的、已提交的 Repository-managed Command，用于：

- Public SDK 与 Private Runner Unit/Contract Test；
- Deterministic Project Archive Generation 与 Hostile-archive Fixture；
- Runtime/Wheelhouse Signature 与 Lock Test；
- Runner Protocol、Cancellation、Restart、Resource 与 Redaction Test；
- Python Factor Exact Golden 与 Repeatability Test；
- Qlib Ridge Artifact/Reload、Withheld-label、Tolerance 与 Forecast Dataset Test；
- 双语 Documentation Path/Parameter/Expected-structure Check；
- 保留 Diagnostic Secret/Path Scan。

每个命令都必须 Exit Zero。记录精确 Test Count、Ignored Test、Warning、Fixture Hash 与任何 Platform-specific Limitation。尚未提交的命令代表 Acceptance Criterion 未满足，不授权虚构本地证据。

## 15. 支持平台 CI

持续要求的 Matrix：

| Trigger | macOS ARM64 | Windows x86_64 | Linux x86_64 |
| --- | --- | --- | --- |
| Pull Request | Fast Manifest/Archive/SDK Contract | Fast Manifest/Archive/SDK Contract | Fast Contract 加完整 Offline Factor → Model Tutorial |
| `main` | Runtime Prepare、适用 Full Chain、Golden 与 Failure | 相同 | 相同 |
| Release/Manual | Runtime Prepare、适用 Full Chain、Golden 与 Failure | 相同 | 相同 |
| 验收 M12 Slice | 为新增 M12 Capability 记录一次 All-platform Green Run | 必需 | 必需 |
| 验收 M13/M14 Slice | 为其 Extension 记录一次 All-platform Green Run | 必需 | 必需 |

完整 Failure Matrix 包含 Cancellation、Untrusted Revision、Lock/Hash Failure、Invalid Output、Restart Recovery 与 Staging Isolation。记录 Workflow URL、精确 SHA、每个 Job URL、Conclusion、Runtime/Wheel Hash 与 Fixture Hash。本地运行不能替代这些证据。

## 16. M12 Slice 验收矩阵

| Slice | 必需的独立证据 | 明确 Out of Scope |
| --- | --- | --- |
| M12.1 Project/Archive/SDK | Rust Core Contract、两个 Python Package Shape、精确 Manifest Validation、安全 Archive Import/Export、Licence、Source-visible Example、聚焦 Hostile-input Test | Runtime Download、Python Execution、Factor/Model Result |
| M12.2 Runtime/Environment | Managed CPython 3.12、Signed Base Wheelhouse、Sync/Lock、Atomic Preparation、Cache Accounting/Eviction、Failure/Retry Test | Research Execution、Custom Interpreter、sdist Build |
| M12.3 Runner/Lifecycle | Private Process、Handshake、IPC/Staging、Trust、Resource、Queue、Cancellation/Restart/Late-result/Redaction Evidence | Factor/Model Semantic Evaluation、Process Pool、Strong Sandbox Claim |
| M12.4 Python Factor | 第三种 Candidate Source、Schema/Reset、精确 Dataset、复用既有 M11 Evidence、Repeatability、Factor Lab/Example/Docs | Qlib Model、Strategy、Component Generation |
| M12.5 Qlib Ridge Core | Host-fed Dataset Bridge、Train-only Transformation、Ridge Adapter、Withheld Test Label、Data-only Artifact/Reload、Unsupported-model Gate | Model Lab Completion、ONNX、Local Qlib Paper |
| M12.6 Model Lab | Grid/Trial、Selection/Final、Forecast Dataset/Evaluation、Repeatability Tolerance、Model Example/Docs 与 Responsive UI | Strategy Execution、Component Generation |
| M12.7 Acceptance | Guided Factor/Model Tutorial、Synthetic Fixture、双语 Docs、Golden/Failure Matrix、All-platform Evidence 与 Final Criterion Mapping | M13 Strategy Completion、M14 Build/Import、Marketplace |

每个 Child Issue 必须把每条 Acceptance Criterion 映射到聚焦实现证据、广泛回归证据、人工证据、精确 Revision、支持平台结果与剩余限制，方可关闭。Child 完成从不授权关闭 Parent Issue，除非 User 明确要求。

## 17. M12 最终验收条件

只有满足以下条件，M12 才被验收：

- 七个有依赖顺序的 Slice 全部完成，并有一张可追踪 Issue Graph；
- 上述每个 M12 项通过，M13/M14 项真实保持延期；
- Factor/Model Example 可执行、双语、Preparation 后 Offline，并可独立检查；
- Synthetic Fixture、Exact Golden Evidence、Ridge Tolerance、Trust、Promotion、Selection 与 Held-out Boundary 均可见；
- Partial、Divergent、Overlapping、Untrusted、Unsupported 或 Failed Result 均未被呈现为 Qualified；
- 聚焦、Workspace、Frontend、Documentation、Secret/Redaction 与三平台 Gate 在被审查 Revision 通过；
- 英文 Completion Evidence 记录 Command、Count、SHA、CI Link、Limitation，且不包含 Secret Material。

仅批准规划或拥有本文档，都不构成实现或验收证据。
