# Host 提供数据的 Qlib Ridge 收益模型

这是用于 ADAQ M12 离线教程的 Apache-2.0、可查看源代码的**合成演示**项目。
注册的 Host 提供 Qlib Ridge Adapter 负责五个 Bar 的连续未来收盘收益目标、仅训练集
变换、`alpha={0.1,1,10}` 网格、无 pickle 的 Linear Model Artifact 以及 Host 计算的预测证据。

项目不包含 Provider、Qlib 数据目录、数据、凭据、Runtime、Environment 或结果。Python 入口
只声明模型契约，合格训练由 Adapter 负责。
