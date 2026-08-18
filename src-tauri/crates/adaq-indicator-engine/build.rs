use std::{env, path::PathBuf};

use sha2::{Digest, Sha256};

#[cfg(feature = "backend-c")]
const SOURCE_SHA256: &str = "40e7a6978052fe5245771e430e6a4c4553b40038f8ac5a985a1540c4c1fa6ace";
const CATALOG_VERSION: &str = "adaq-indicator-catalog@1.0.0";
#[cfg(feature = "backend-c")]
const XML_SHA256: &str = "70ed7629a577cb3803ed2882607070beb15592724ea4366735a9e0fc8413dec1";
#[cfg(feature = "backend-c")]
const ABSTRACT_HEADER_SHA256: &str =
    "babd4a971b3f404937b77bafaef3a34d5ce92370b0f5cf7de8917a1716bb394a";
#[cfg(feature = "backend-c")]
const FUNCTION_HEADER_SHA256: &str =
    "c4308ddbd0f17597051e3910ad59b84cc6fd4f1991bacb0680210cc310d35634";

// Version of the pure-Rust `adaq-talib` backend. Kept in sync with the dependency in
// Cargo.toml; used to derive a stable build id for the rust backend.
const ADAQ_TALIB_VERSION: &str = "0.1.8";

struct Env {
    ta_source_sha256: String,
    wrapper_sha256: String,
    compiler_and_flags_sha256: String,
    talib_version: String,
}

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let target = env::var("TARGET").unwrap();

    let env = compute_env(&manifest_dir, &target);

    // Build id: a stable sha256 used by downstream provenance checks. Identical inputs
    // (backend + library version + catalog + target) must produce the same id.
    let build_id = hex(&Sha256::digest(
        [
            if env.ta_source_sha256 == "rust" {
                "rust"
            } else {
                "c"
            }
            .as_bytes(),
            CATALOG_VERSION.as_bytes(),
            ADAQ_TALIB_VERSION.as_bytes(),
            target.as_bytes(),
            env.compiler_and_flags_sha256.as_bytes(),
        ]
        .concat(),
    ));

    println!(
        "cargo:rustc-env=ADAQ_INDICATOR_ENGINE_TA_SOURCE_SHA256={}",
        env.ta_source_sha256
    );
    println!(
        "cargo:rustc-env=ADAQ_INDICATOR_ENGINE_WRAPPER_SHA256={}",
        env.wrapper_sha256
    );
    println!("cargo:rustc-env=ADAQ_INDICATOR_ENGINE_TARGET={target}");
    println!(
        "cargo:rustc-env=ADAQ_INDICATOR_ENGINE_COMPILER_AND_FLAGS_SHA256={}",
        env.compiler_and_flags_sha256
    );
    println!("cargo:rustc-env=ADAQ_INDICATOR_ENGINE_BUILD_ID={build_id}");
    println!("cargo:rustc-env=ADAQ_INDICATOR_ENGINE_TALIB_VERSION={}", env.talib_version);

    println!("cargo:rerun-if-changed=src/catalog.rs");
    println!("cargo:rerun-if-env-changed=CC");
    println!("cargo:rerun-if-env-changed=CFLAGS");
}

#[cfg(feature = "backend-c")]
fn compute_env(manifest_dir: &PathBuf, _target: &str) -> Env {
    use std::{fs, process::Command};

    let archive = manifest_dir.join("vendor/ta-lib-0.7.1.tar.gz");
    let source = fs::read(&archive).unwrap();
    assert_eq!(
        hex(&Sha256::digest(&source)),
        SOURCE_SHA256,
        "TA-Lib source checksum changed"
    );
    println!("cargo:rerun-if-changed=vendor/ta-lib-0.7.1.tar.gz");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let source_root = out_dir.join("ta-lib-0.7.1");
    if !source_root.exists() {
        tar::Archive::new(flate2::read::GzDecoder::new(source.as_slice()))
            .unpack(&out_dir)
            .unwrap();
    }
    for (path, expected) in [
        ("ta_func_api.xml", XML_SHA256),
        ("include/ta_abstract.h", ABSTRACT_HEADER_SHA256),
        ("include/ta_func.h", FUNCTION_HEADER_SHA256),
    ] {
        assert_eq!(
            hex(&Sha256::digest(fs::read(source_root.join(path)).unwrap())),
            expected,
            "TA-Lib metadata checksum changed: {path}"
        );
    }

    let destination = cmake::Config::new(&source_root)
        .define("BUILD_DEV_TOOLS", "OFF")
        .profile("Release")
        .build_target("ta-lib-static")
        .build();
    println!(
        "cargo:rustc-link-search=native={}",
        destination.join("build").display()
    );
    if env::var("CARGO_CFG_TARGET_OS").unwrap() == "windows" {
        println!(
            "cargo:rustc-link-search=native={}",
            destination.join("build/Release").display()
        );
        println!("cargo:rustc-link-lib=static=ta-lib-static");
    } else {
        println!("cargo:rustc-link-lib=static=ta-lib");
        println!("cargo:rustc-link-lib=m");
    }

    let cache = fs::read_to_string(destination.join("build/CMakeCache.txt")).unwrap();
    let compiler = cmake_cache_value(&cache, "CMAKE_C_COMPILER").unwrap_or("cc");
    let compiler_identity = Command::new(compiler)
        .arg("--version")
        .output()
        .map(|output| {
            format!(
                "{compiler}\n{}\n{}\n{}",
                String::from_utf8_lossy(&output.stdout),
                String::from_utf8_lossy(&output.stderr),
                output.status
            )
        })
        .unwrap_or_else(|_| compiler.into());
    let wrapper = fs::read(manifest_dir.join("src/bindings.rs")).unwrap();
    let build_flags = [
        cmake_cache_value(&cache, "CMAKE_C_FLAGS")
            .unwrap_or_default()
            .to_owned(),
        cmake_cache_value(&cache, "CMAKE_C_FLAGS_RELEASE")
            .unwrap_or_default()
            .to_owned(),
        env::var("CFLAGS").unwrap_or_default(),
    ]
    .join(";");
    let compiler_and_flags_sha256 = hex(&Sha256::digest(
        [compiler_identity.as_bytes(), build_flags.as_bytes()].concat(),
    ));
    let wrapper_sha256 = hex(&Sha256::digest(&wrapper));

    Env {
        ta_source_sha256: SOURCE_SHA256.to_string(),
        wrapper_sha256,
        compiler_and_flags_sha256,
        talib_version: "0.7.1".to_string(),
    }
}

#[cfg(not(feature = "backend-c"))]
fn compute_env(_manifest_dir: &PathBuf, _target: &str) -> Env {
    Env {
        ta_source_sha256: "rust".to_string(),
        wrapper_sha256: "adaq-talib".to_string(),
        compiler_and_flags_sha256: "rust".to_string(),
        talib_version: ADAQ_TALIB_VERSION.to_string(),
    }
}

#[cfg(feature = "backend-c")]
fn cmake_cache_value<'a>(cache: &'a str, key: &str) -> Option<&'a str> {
    cache.lines().find_map(|line| {
        line.strip_prefix(key)?
            .split_once('=')
            .map(|(_, value)| value)
    })
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}
