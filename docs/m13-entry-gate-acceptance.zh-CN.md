# M13 入口门验收记录

状态：**自动化门已通过；产品运行验收待执行**

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
| 三市场 acquisition 可见性 | Rust 生命周期测试与 Data Foundation UI 测试 | 待执行 |
| 取消与重试 | Rust 生命周期测试与持久化 operation ledger | 待执行 |
| Host 重启恢复 | Rust 重启恢复测试 | 待执行 |
| 证据留存与质量 fail-closed | Pipeline、Snapshot、Context 测试 | 待执行 |
| Features → Factors → Models Context handoff | Context freeze 与 attempt binding 测试 | 待执行 |
| stale、混市场、不完整、无权限证据拒绝 | Context contract 测试 | 待执行 |
| Data Foundation 中英文界面 | i18n 测试与 UI 测试 | 待执行 |

## 产品运行证据

针对 OKX、中国 A 股、美国股票分别记录 operation ID、Market、Venue、状态迁移、时间戳、可见错误和保留证据。执行成功、取消、失败后重试、运行中 Host 重启。随后在 Data Foundation 选择已发布 Snapshot 与 Point-in-Time Universe，建立 Context，并在 Features、Factors、Models 各冻结一次，记录冻结 revision、operation ID 与 lineage。再使用 stale、混市场、不完整及其他用户的证据，记录界面展示的 typed blocker。

本节需要 Provider 凭据与桌面 GUI 运行。自动化测试继续作为确定性契约行为的依据。
