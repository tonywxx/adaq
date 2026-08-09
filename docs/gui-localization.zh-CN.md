# GUI 多语言指南

[English](./gui-localization.md)

状态：V1 用户、文案编写与验收契约。

## V1 语言契约

V1 只内置两套翻译资源：

| Resource Locale | 用户看到的名称 | 用途 |
| --- | --- | --- |
| `en-US` | English (US) | 默认及回退 GUI 文案。 |
| `zh-CN` | 简体中文 | 完整简体中文 GUI 文案。 |

Settings > General 提供三个选项，但 System 是自动解析方式，不是第三种语言：

| 选项 | 解析结果 |
| --- | --- |
| System | 系统语言为中文时解析为 `zh-CN`；其他系统语言解析为 `en-US`。 |
| English (US) | 始终使用 `en-US`。 |
| 简体中文 | 始终使用 `zh-CN`。 |

不受支持的系统语言不会触发运行时机器翻译或下载语言包，而是使用完整 `en-US` 资源。

## 设置行为

- 新设备默认选择 System。
- 选择结果作为设备本地 Interface Locale Preference 保存，不属于 User Profile 数据。
- 修改后立即生效，无需重启或重新登录。
- 应用必须在首个可见 Paint 前解析并初始化语言，避免先闪现另一种语言。
- 退出登录或重置用户 Research Data 后仍保留该选择。
- 当前生效的 Resource Locale 必须同步更新文档 `lang` 属性，供辅助技术正确识别。

## 已实现的基础能力

当前基础能力集中在 `src/lib/i18n.ts`，并由 `src/main.tsx` 在 React 渲染前导入。它使用 `adaq.interfaceLocale` 保存设备本地偏好，在运行时切换语言时保持当前路由不变，并提供日期、数字和精确 Decimal 展示所需的共享 `Intl` 辅助函数。应用 Shell、Navigation、Authentication/Loading Primitive 和 Settings > General 共用同一套资源；语言初始化不依赖 Native IPC。

## 哪些内容必须翻译

所有面向用户的 GUI 都必须来自翻译资源，包括：

- Navigation、Page Heading、Tab、Card、Table、Empty State 与 Dashboard Label。
- Button、Menu、Form、Placeholder、Validation Message 与 Confirmation Dialog。
- Loading、Progress、Success、Warning、Error、Connection、Reconciliation 与 Bot State Summary。
- Tooltip、Accessible Name、Image Alternative、Keyboard Instruction 与 Screen-reader-only Text。
- 面向用户的 Factor、Model、Strategy、Metric、Risk Decision 与 Execution Evidence 说明。

插值、复数、日期、数量和值必须放在完整译句内。代码不得拼接依赖英文词序的翻译片段来组装句子。

## 多语言绝不能改变什么

Interface Locale 只改变显示，绝不能改变以下内容的身份或存储语义：

- Instrument ID、Venue Code、Ticker、Component ID、Version、Hash、Enum Wire Value 与 Schema Field。
- 精确 Decimal Price、Quantity、Balance、Rate、Metric、Timestamp、Trading Date 与 Venue Time Zone。
- Market Data Snapshot、Research Protocol、Model Artifact、Evidence Payload、Provider Response、Log 与 Export。
- 用户创建的 Name、Note、Source Code、Model Label 与导入 Component Metadata。

界面可以在 Canonical Value 旁显示翻译标签。Provider Error 和 Technical Diagnostic 必须保留原始细节；ADAQ 可以增加翻译后的分类和恢复说明，但不得改写原始证据。

## 格式化规则

日期、数字、百分比和货币使用平台原生 `Intl` API，并传入当前解析后的 Resource Locale。格式化绝不能改变底层精确值：

- `en-US` 与 `zh-CN` 可以用不同形式显示同一个 Instant 或 Decimal。
- Venue-local Market Time 仍由 Trading Calendar 与 IANA Venue Time Zone 决定，不受 Interface Locale 控制。
- Currency Display 必须保留真实 Currency Code 或无歧义 Symbol。
- 格式化后的 Display String 不能被重新用于 Canonical ID、Hash Input、Serialized Value 或 Calculation Input。

## 翻译资源规则

- `i18next` 负责 Locale Resolution、Fallback、Interpolation 与 Resource Lookup；React 界面使用 `react-i18next`。
- 翻译资源随 Desktop Application 一起打包，离线可用。
- Translation Key 必须语义稳定，例如 `settings.general.language.title`，不能直接把英文句子作为 Key。
- `en-US` 是 Fallback Locale；缺失 Key 时必须显示有效英文文案，不能显示空白标签。
- 英文与中文资源必须拥有相同 Key 和 Interpolation Variable。
- Domain Enum 在存储中保持 Canonical，只在 GUI Boundary 映射为翻译后的 Display Label。
- 即使优先显示翻译摘要，也必须允许用户查看原始 Technical Evidence。

## 验收检查

V1 验收前必须完成：

1. 自动检查 `en-US` 与 `zh-CN` 的 Key 和 Interpolation Variable 一致性。
2. 分别直接以两个显式语言和 System 模式启动应用，确认首屏没有语言闪烁。
3. 用两种语言检查全部交付 Route，包括 Loading、Empty、Degraded、Failure 与 Confirmation State。
4. 在运行时切换语言，并在 Restart 与 Sign-out 后验证持久化结果。
5. 检查 Table、Chart、Dialog、Title Bar、窄布局以及较长中英文文案是否截断或产生不可访问 Overflow。
6. 用两种语言验证 Keyboard 与 Screen Reader Label。
7. 比较两种语言下生成的 Canonical Export；除明确允许本地化的展示文档外，结果必须等价。

## 以后增加其它语言

新增语言属于 V1 之后的工作。它必须具备完整 Resource、Key 与 Interpolation Parity、Locale-aware Formatting Review、全部 Route 的 Layout 与 Accessibility 验收、对应用户文档以及明确的 System Resolution Rule。新增语言不需要改变 ADAQ 的 Domain Record 或 Evidence Format。
