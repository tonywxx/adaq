//! 构建脚本:从 `src/exchange.rs` 自动提取统一方法面清单(ADR-0001)。
//!
//! 生成 `OUT_DIR/methods.rs`:
//! - `REST_METHODS`:Exchange trait 的 REST 方法名(含显式 `async fn` 与
//!   `def_method!` 宏生成的方法),按源码出现顺序。
//! - `WS_METHODS`:Realtime trait 的 watch_* 方法名。
//!
//! 清单与 trait 同源,避免手维护漂移;`rerun-if-changed` 保证 trait 变动时重建。

use std::path::Path;

use regex::Regex;

fn main() {
    println!("cargo:rerun-if-changed=src/exchange.rs");

    let src = std::fs::read_to_string("src/exchange.rs").expect("read src/exchange.rs");

    // 定位两个 trait 的文本区间
    let exchange_start = src
        .find("pub trait Exchange")
        .expect("pub trait Exchange not found");
    let realtime_start = src
        .find("pub trait Realtime")
        .expect("pub trait Realtime not found");
    let tests_start = src.find("#[cfg(test)]").unwrap_or(src.len());

    let rest_section = &src[exchange_start..realtime_start];
    let ws_section = &src[realtime_start..tests_start];

    let method_re = Regex::new(r"(?:def_method!\s*\(\s*|async fn\s+)([a-z_][a-z0-9_]*)").unwrap();

    fn collect(section: &str, re: &Regex) -> Vec<String> {
        let mut names = Vec::new();
        for cap in re.captures_iter(section) {
            names.push(cap[1].to_string());
        }
        names
    }

    let rest = collect(rest_section, &method_re);
    let ws = collect(ws_section, &method_re);

    // 生成清单文件
    let mut out = String::new();
    out.push_str("// 本文件由 build.rs 自动生成,请勿手改。\n");
    out.push_str("// 来源:src/exchange.rs(Exchange / Realtime trait)。\n\n");
    out.push_str(&format!(
        "pub const REST_METHODS: &[&str] = &[\n    {},\n];\n\n",
        rest.iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(",\n    ")
    ));
    out.push_str(&format!(
        "pub const WS_METHODS: &[&str] = &[\n    {},\n];\n\n",
        ws.iter()
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(",\n    ")
    ));
    out.push_str("pub const ALL_METHODS: &[&str] = &[\n    ");
    out.push_str(
        &rest
            .iter()
            .chain(ws.iter())
            .map(|n| format!("\"{n}\""))
            .collect::<Vec<_>>()
            .join(", "),
    );
    out.push_str(",\n];\n");

    let out_path = Path::new(&std::env::var("OUT_DIR").unwrap()).join("methods.rs");
    std::fs::write(&out_path, out).expect("write methods.rs");

    // Candidate 4(ADR-0001):扫描 curated 适配器,生成注册面与契约 pairs。
    gen_adapter_registration();

    eprintln!(
        "methods manifest: {} REST + {} WS methods",
        rest.len(),
        ws.len()
    );
}

/// 扫描 `src/adapters/*.rs`(curated,排除 mod/outcome/generated 与 generated/ 子目录),
/// 提取 `impl Exchange for X` 与 `pub const ID`,生成两份产物:
///
/// - `OUT_DIR/adapter_reg.rs`:`#[cfg(feature)] pub mod X` + `pub use X::{...}`,
///   供 `src/adapters/mod.rs` 通过 `include!` 吸收 —— 删除手维护的成对注册。
/// - `OUT_DIR/contract_pairs.rs`:`pub const ADAPTER_PAIRS: &[(&str, &[&str])]`,
///   供 `src/contract_gen.rs` 通过 `include!` 吸收 —— 删除 `tests/contract.rs` 手抄的 pairs。
///
/// 真源在适配器自身的 `ID` / `IMPLEMENTED` 常量;机械注册簇被删除,漂移面随之消失
/// (对照上面已从 `exchange.rs` 抽取方法清单的先例)。
fn gen_adapter_registration() {
    use std::collections::HashSet;

    let adapters_dir = Path::new("src/adapters");
    println!("cargo:rerun-if-changed=src/adapters");
    println!("cargo:rerun-if-changed=Cargo.toml");

    // 已知非适配器模块,跳过(它们不是 curated 交易所注册面的一部分)。
    // `adapter_reg` 是本案自身生成的产物,也须排除,否则会被误当作适配器扫描。
    let skip: HashSet<&str> = ["mod", "outcome", "generated", "adapter_reg"]
        .iter()
        .copied()
        .collect();

    let mut entries: Vec<_> = std::fs::read_dir(adapters_dir)
        .expect("read src/adapters")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().map(|x| x == "rs").unwrap_or(false))
        .filter(|p| {
            p.file_stem()
                .and_then(|s| s.to_str())
                .map(|s| !skip.contains(s))
                .unwrap_or(false)
        })
        .collect();
    entries.sort();

    let struct_re = Regex::new(r"impl\s+Exchange\s+for\s+([A-Z][A-Za-z0-9]+)").unwrap();
    let id_re = Regex::new(r"pub\s+const\s+ID\s*[:=]").unwrap();

    // (module, struct, has_id)
    let mut adapters: Vec<(String, String, bool)> = Vec::new();
    for path in entries {
        let src = std::fs::read_to_string(&path).expect("read adapter");
        let module = path.file_stem().unwrap().to_string_lossy().to_string();
        let struct_name = struct_re
            .captures(&src)
            .map(|c| c[1].to_string())
            .unwrap_or_else(|| panic!("未找到 `impl Exchange for X`: {}", path.display()));
        let has_id = id_re.is_match(&src);
        adapters.push((module, struct_name, has_id));
    }

    // 读取 Cargo.toml 用于:(a) 漂移守卫所需的 feature 名集合;(b) 后续可选校验。
    let cargo = std::fs::read_to_string("Cargo.toml").expect("read Cargo.toml");

    // 全部 feature 名,用于漂移守卫(适配器存在但 Cargo.toml 缺同名 feature)。
    // 只在 `[features]` 段内匹配,避免误吞 `[dependencies]` 等段的键名。
    let feat_pos = cargo.find("[features]").unwrap_or(0);
    let end = cargo[feat_pos..]
        .find("\n[")
        .map(|i| feat_pos + i)
        .unwrap_or(cargo.len());
    let features_section = cargo.get(feat_pos..end).unwrap_or("").to_string();
    let feature_names: HashSet<String> = Regex::new(r"(?m)^([a-zA-Z][\w-]*)\s*=")
        .unwrap()
        .captures_iter(&features_section)
        .map(|c| c[1].to_string())
        .collect();

    // --- 生成 adapter_reg.rs(mod.rs include) ---
    // 关键点:`include!` 把文本拼接到 `src/adapters/mod.rs`,但其中的 `mod X;`
    // 仍按「被包含文件自身所在目录」解析(即 OUT_DIR),而非 mod.rs 所在目录。
    // 因此必须显式用 `#[path]` 指向真实适配器文件 `src/adapters/X.rs`,否则
    // 把文件放进 OUT_DIR 后 Rust 会去 OUT_DIR 找 X.rs 而失败。
    // `#[path]` 用绝对路径(来自 CARGO_MANIFEST_DIR),构建时重新生成,发布包
    // 不含本文件,故不受机器路径差异影响。
    let adapters_root =
        Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap()).join("src/adapters");
    let mut reg = String::new();
    reg.push_str("// 本文件由 build.rs 自动生成,请勿手改。\n");
    reg.push_str(
        "// 来源:扫描 src/adapters/*.rs 提取 `impl Exchange for X` 与 `pub const ID`。\n\n",
    );
    for (module, struct_name, has_id) in &adapters {
        let cfg = format!("#[cfg(feature = \"{module}\")]");
        let abs = format!("{:?}", adapters_root.join(format!("{module}.rs")));
        reg.push_str(&format!("{cfg} #[path = {abs}] pub mod {module};\n"));
        if *has_id {
            let alias = format!("{}_ID", module.to_uppercase());
            reg.push_str(&format!(
                "{cfg} pub use {module}::{{{struct_name}, ID as {alias}}};\n"
            ));
        } else {
            reg.push_str(&format!("{cfg} pub use {module}::{struct_name};\n"));
        }
    }

    // --- 生成 contract_pairs.rs(lib crate 内 const,供 contract.rs 消费) ---
    // 全部条目均按 feature 门控:即使 `--no-default-features` 也能编译
    // (`contract_gen` 是常驻 lib 模块),且 default feature 在默认构建下自然在场。
    let mut pairs = String::new();
    pairs.push_str("// 本文件由 build.rs 自动生成,请勿手改。\n");
    pairs.push_str("// 来源:扫描 src/adapters/*.rs。\n");
    pairs.push_str("pub const ADAPTER_PAIRS: &[(&str, &[&str])] = &[\n");
    for (module, struct_name, _has_id) in &adapters {
        let path = format!("crate::adapters::{struct_name}");
        pairs.push_str(&format!(
            "    #[cfg(feature = \"{module}\")] (\"{module}\", {path}::IMPLEMENTED),\n"
        ));
    }
    pairs.push_str("];\n");

    // adapter_reg.rs 写入 OUT_DIR:`include!` 中的 `mod X;` 仍相对于本文件目录
    // 解析,故上面的 `#[path]` 已显式指向 `src/adapters/X.rs`。放进 OUT_DIR 才能
    // 通过 `cargo publish --verify` —— 构建脚本禁止改写源码树(OUT_DIR 之外),
    // 否则发布校验直接失败。发布包不含本文件(构建时重新生成)。
    let out_dir = std::env::var("OUT_DIR").unwrap();
    std::fs::write(Path::new(&out_dir).join("adapter_reg.rs"), reg).expect("write adapter_reg.rs");

    std::fs::write(Path::new(&out_dir).join("contract_pairs.rs"), pairs)
        .expect("write contract_pairs.rs");

    // 漂移守卫:适配器文件存在但 Cargo.toml 无同名 feature
    for (module, _, _) in &adapters {
        if !feature_names.contains(module) {
            println!(
                "cargo:warning=适配器模块 `{module}` 在 src/adapters/ 存在,但 Cargo.toml 缺少同名 feature"
            );
        }
    }

    eprintln!(
        "adapter registration: {} curated adapters scanned",
        adapters.len()
    );
}
