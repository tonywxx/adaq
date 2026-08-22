# M13 入口门验收记录

状态：**自动化门已通过；仅 OKX 的产品运行验收进行中；A 股和美股验收延期**

## 当前交付范围

在找到稳定的 A 股和美国股票数据源之前，端到端开发与产品运行验收只推进 OKX Spot 路径。A 股和美股路径在本 Delivery Slice 中明确标记为 **Not Tested / Deferred**。这是临时范围延期，不是 V1 readiness 结论，也不代表删除三市场目标。

本记录关闭 M13 前置任务的可执行部分。桌面 GUI 与 Provider 流程仍需由验收人员完成并填写下表。

## 自动化证据

| 检查 | 结果 |
|---|---|
| `cargo test --workspace` | 通过 |
| `cargo check --workspace` | 通过 |
| `pnpm exec jest --watchman=false --runInBand` | 通过：37 suites、120 tests |
| `pnpm run build` | 通过 |
| Biome focused check | 通过 |
| `git diff --check` | 通过 |
| `pnpm tauri dev` | 桌面 binary 已编译并启动；交互应用保持运行，命令因 smoke-test 超时退出 |

## 入口门矩阵

| 门 | 自动化覆盖 | 产品运行 |
|---|---|---|
| OKX acquisition 可见性 | Rust 生命周期测试与 Data Foundation UI 测试 | 待执行：仅 OKX 流程 |
| OKX 取消与重试 | Rust 生命周期测试与持久化 operation ledger | 待执行：仅 OKX 流程 |
| OKX Host 重启恢复 | Rust 重启恢复测试 | 待执行：仅 OKX 流程 |
| OKX 证据留存与质量 fail-closed | Pipeline、Snapshot、Context 测试 | 待执行：仅 OKX 流程 |
| OKX Features → Factors → Models Context handoff | Context freeze 与 attempt binding 测试 | 待执行：仅 OKX 流程 |
| OKX stale、混市场、不完整、无权限证据拒绝 | Context contract 测试 | 待执行：仅 OKX 流程 |
| OKX Data Foundation 中英文界面 | i18n 测试与 UI 测试 | 当前观察界面通过；OKX 流程仍待完成 |
| A 股和美股产品运行验收 | Provider-backed GUI 运行 | Not Tested / Deferred |

## 产品运行证据

当前 Delivery Slice 只针对 OKX Spot 记录 operation ID、Market、Venue、状态迁移、时间戳、可见错误和保留证据。执行成功、取消、失败后重试、运行中 Host 重启。随后在 Data Foundation 选择已发布 Snapshot 与 Point-in-Time Universe，建立 Context，并在 Features、Factors、Models 各冻结一次，记录冻结 revision、operation ID 与 lineage。A 股和美股场景在稳定数据源可用前标记为 Not Tested / Deferred。

本节需要 Provider 凭据与桌面 GUI 运行。自动化测试继续作为确定性契约行为的依据。

## 探索性观察（不计入当前验收）

在 commit `2144131` 的 macOS 桌面 GUI 中完成检查，使用已认证的 ADAQ 会话。此处不记录凭据、Token、私有路径或完整 Provider 响应。

| 场景 | 证据 | 结果 |
|---|---|---|
| OKX acquisition 成功 | `crypto-foundation-997eacda-9824-4a30-822a-1c68b98a94ac` | Completed；OKX evidence 在 ledger 中可见。 |
| A 股取消 | `a-shares-foundation-f08a0c4f-dfe7-4bb1-9872-e76363902d71` | Cancelled；刷新 GUI 后可见保留的 typed error。 |
| A 股重试/Provider 失败 | `a-shares-foundation-257b0c80-7ee5-47a0-8b9b-dbb3f1fbe81c` | 以 typed Provider decode error 失败，并保留 response hash；没有错误授予 readiness。 |
| A 股失败后重试 | `a-shares-foundation-c1def5c9-e885-4a6c-a8f0-83700fd50f4f` | 重试创建了新的 operation，并保留相同的 typed Provider decode failure。 |
| 美国股票当前 Provider 路径 | `us-equities-foundation-64e25661-709b-48d2-9d2f-ec5295988f04` | 以 typed `not_found` 失败；当前 Yahoo 路径没有产生股票 universe。此前有一次 Alpaca operation 成功，但不是当前路径的新鲜成功证据。 |
| Host restart recovery | 第二次 A 股尝试被终止，但 Provider 在终止前已失败，未能观察到 in-flight restart 场景。 | 未证明；仍待验收。 |
| Context 选择与冻结 | Data Foundation 没有 published Snapshot 或 Point-in-Time Universe；`Establish Context` 为 disabled。 | 在 Features → Factors → Models 冻结前被阻塞。 |
| 简体中文 GUI | 在 Settings 切换为 `zh-CN`；Data Foundation 的标签、状态和重试控件仍可见并已本地化。 | 当前观察界面通过。 |

上面的 A 股和美股探索性观察只用于说明延期原因，不作为当前 OKX-only 范围的验收证据。当前产品运行验收仍为 **Pending**，直到完整 OKX 流程完成。
