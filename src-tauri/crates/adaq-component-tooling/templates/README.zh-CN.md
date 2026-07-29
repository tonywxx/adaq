# {{display_name}}

本项目是由 `adaq-component` 生成的 ADAQ 组件。

## 开发

编辑 `src/lib.rs` 来实现组件，编辑 `manifest.json` 来声明其稳定的元数据。金融值为十进制字符串；在需要分析性 `f64` 输出之前保持其精确性。

安装 Rust、`cargo-component` 和独立的 `adaq-component` CLI，然后运行：

```sh
adaq-component build
adaq-component verify dist/{{name}}-0.1.0.adaq
```

`build` 运行测试、编译 release WebAssembly 组件、验证合规性，并将不可变的包写入 `dist/`。从 ADAQ 组件库导入该 `.adaq` 文件。组件代码在此 Rust 项目中编辑；Tauri 应用不提供 GUI 代码编辑器。
