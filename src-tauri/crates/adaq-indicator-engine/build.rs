use std::{env, fs, path::PathBuf, process::Command};

use sha2::{Digest, Sha256};

const SOURCE_SHA256: &str = "40e7a6978052fe5245771e430e6a4c4553b40038f8ac5a985a1540c4c1fa6ace";

fn main() {
    let manifest_dir = PathBuf::from(env::var("CARGO_MANIFEST_DIR").unwrap());
    let archive = manifest_dir.join("vendor/ta-lib-0.7.1.tar.gz");
    let source = fs::read(&archive).unwrap();
    assert_eq!(
        hex(&Sha256::digest(&source)),
        SOURCE_SHA256,
        "TA-Lib source checksum changed"
    );
    println!("cargo:rerun-if-changed={}", archive.display());
    println!("cargo:rerun-if-changed=src/bindings.rs");

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let source_root = out_dir.join("ta-lib-0.7.1");
    if !source_root.exists() {
        tar::Archive::new(flate2::read::GzDecoder::new(source.as_slice()))
            .unpack(&out_dir)
            .unwrap();
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
    println!("cargo:rustc-link-lib=static=ta-lib");
    if env::var("CARGO_CFG_TARGET_OS").unwrap() != "windows" {
        println!("cargo:rustc-link-lib=m");
    }

    let cache = fs::read_to_string(destination.join("build/CMakeCache.txt")).unwrap();
    let compiler = cmake_cache_value(&cache, "CMAKE_C_COMPILER").unwrap_or("cc");
    let compiler_identity = Command::new(&compiler)
        .arg("--version")
        .output()
        .ok()
        .map(|output| String::from_utf8_lossy(&output.stdout).into_owned())
        .unwrap_or_else(|| compiler.into());
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
    let build_id = hex(&Sha256::digest(
        [
            SOURCE_SHA256.as_bytes(),
            &wrapper,
            env::var("TARGET").unwrap().as_bytes(),
            compiler_identity.as_bytes(),
            build_flags.as_bytes(),
            b"CMAKE_BUILD_TYPE=Release;BUILD_DEV_TOOLS=OFF",
        ]
        .concat(),
    ));
    println!("cargo:rustc-env=ADAQ_INDICATOR_ENGINE_BUILD_ID={build_id}");
}

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
