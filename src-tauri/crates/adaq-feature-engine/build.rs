use std::{env, fs, path::PathBuf, process::Command};

use sha2::{Digest, Sha256};

const FEATURE_ENGINE_VERSION: &str = "adaq-feature-engine@1.0.0";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let source_paths = [
        "Cargo.toml",
        "build.rs",
        "src/lib.rs",
        "src/execution.rs",
        "../../Cargo.lock",
        "../adaq-data-core/Cargo.toml",
        "../adaq-data-core/src/lib.rs",
        "../adaq-data-core/src/market.rs",
        "../adaq-data-core/src/a_share.rs",
    ];
    for path in source_paths {
        println!(
            "cargo:rerun-if-changed={}",
            manifest_dir.join(path).display()
        );
    }

    let source = source_paths
        .into_iter()
        .flat_map(|path| fs::read(manifest_dir.join(path)).unwrap())
        .collect::<Vec<_>>();
    let source_sha256 = hex(&Sha256::digest(&source));
    let target = env::var("TARGET").unwrap();
    let rustc = env::var("RUSTC").unwrap_or_else(|_| "rustc".into());
    let compiler = Command::new(&rustc)
        .args(["--version", "--verbose"])
        .output()
        .map(|output| {
            format!(
                "{rustc}\n{}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                output.status
            )
        })
        .unwrap_or(rustc);
    let compiler_and_flags = format!(
        "{compiler}\ntarget={target}\nprofile={}\ndebug={}\nflags={}",
        env::var("OPT_LEVEL").unwrap_or_default(),
        env::var("DEBUG").unwrap_or_default(),
        env::var("CARGO_ENCODED_RUSTFLAGS").unwrap_or_default()
    );
    let compiler_and_flags_sha256 = hex(&Sha256::digest(compiler_and_flags.as_bytes()));
    let build_id = hex(&Sha256::digest(
        format!("{FEATURE_ENGINE_VERSION}:{source_sha256}:{target}:{compiler_and_flags_sha256}")
            .as_bytes(),
    ));

    println!("cargo:rustc-env=ADAQ_FEATURE_ENGINE_SOURCE_SHA256={source_sha256}");
    println!("cargo:rustc-env=ADAQ_FEATURE_ENGINE_TARGET={target}");
    println!(
        "cargo:rustc-env=ADAQ_FEATURE_ENGINE_COMPILER_AND_FLAGS_SHA256={compiler_and_flags_sha256}"
    );
    println!("cargo:rustc-env=ADAQ_FEATURE_ENGINE_BUILD_ID={build_id}");
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
