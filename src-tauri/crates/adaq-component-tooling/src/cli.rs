use std::{
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    ComponentManifest, ComponentPackage, ComponentTemplate, check_manifest_compatibility,
    create_project, pack_component, verify_package,
};

const USAGE: &str = "usage:\n  adaq-component new <factor|strategy> <name>\n  adaq-component build\n  adaq-component verify <package.adaq> [--previous <manifest.json>]";

pub fn run_cli(arguments: &[String], cwd: &Path) -> Result<(), String> {
    match arguments {
        [command, kind, name] if command == "new" => {
            let sdk_path = env::var_os("ADAQ_COMPONENT_SDK_PATH").map(PathBuf::from);
            let path = create_project(
                ComponentTemplate::parse(kind)?,
                name,
                cwd,
                sdk_path.as_deref(),
            )?;
            println!("created {}", path.display());
            Ok(())
        }
        [command] if command == "build" => {
            let path = build_project(cwd)?;
            println!("built {}", path.display());
            Ok(())
        }
        [command, package] if command == "verify" => verify(cwd, package, None),
        [command, package, flag, previous] if command == "verify" && flag == "--previous" => {
            verify(cwd, package, Some(previous))
        }
        _ => Err(USAGE.into()),
    }
}

fn verify(cwd: &Path, package_path: &str, previous_path: Option<&String>) -> Result<(), String> {
    let package = ComponentPackage::read(&fs::read(cwd.join(package_path)).map_err(string)?)
        .map_err(string)?;
    verify_package(&package)?;
    if let Some(previous_path) = previous_path {
        let previous = serde_json::from_slice(&fs::read(cwd.join(previous_path)).map_err(string)?)
            .map_err(string)?;
        check_manifest_compatibility(&previous, &package.manifest).map_err(string)?;
        println!("confirmed Component SemVer compatibility with {previous_path}");
    }
    println!(
        "verified {} {} ({})",
        package.manifest.name, package.manifest.version, package.archive_sha256
    );
    Ok(())
}

pub fn build_project(root: &Path) -> Result<PathBuf, String> {
    let manifest_path = root.join("manifest.json");
    let manifest: ComponentManifest =
        serde_json::from_slice(&fs::read(&manifest_path).map_err(string)?).map_err(string)?;
    let cargo_toml = fs::read_to_string(root.join("Cargo.toml")).map_err(string)?;
    let crate_name = cargo_package_name(&cargo_toml)?;
    run(
        Command::new("cargo").arg("test").current_dir(root),
        "cargo test",
    )?;
    let path = rust_toolchain_path()?;
    let mut component = Command::new("rustup");
    component
        .args([
            "run",
            "stable",
            "cargo",
            "component",
            "build",
            "--release",
            "--target",
            "wasm32-unknown-unknown",
        ])
        .current_dir(root)
        .env("PATH", path);
    run(&mut component, "cargo component build")?;
    let wasm_path = root
        .join("target/wasm32-unknown-unknown/release")
        .join(format!("{}.wasm", crate_name.replace('-', "_")));
    let bytes = pack_component(manifest, &fs::read(&wasm_path).map_err(string)?).map_err(string)?;
    let package = ComponentPackage::read(&bytes).map_err(string)?;
    verify_package(&package)?;
    let dist = root.join("dist");
    fs::create_dir_all(&dist).map_err(string)?;
    let output = dist.join(format!("{}-{}.adaq", crate_name, package.manifest.version));
    fs::write(&output, bytes).map_err(string)?;
    Ok(output)
}

fn cargo_package_name(cargo_toml: &str) -> Result<&str, String> {
    cargo_toml
        .lines()
        .skip_while(|line| line.trim() != "[package]")
        .skip(1)
        .take_while(|line| !line.trim().starts_with('['))
        .find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == "name").then(|| value.trim().trim_matches('"'))
        })
        .filter(|name| !name.is_empty())
        .ok_or_else(|| "Cargo.toml package name is missing".into())
}

fn run(command: &mut Command, label: &str) -> Result<(), String> {
    let status = command.status().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            format!("{label} is unavailable; install Rust and cargo-component")
        } else {
            error.to_string()
        }
    })?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{label} failed with {status}"))
    }
}

fn rust_toolchain_path() -> Result<OsString, String> {
    let output = Command::new("rustup")
        .args(["which", "--toolchain", "stable", "rustc"])
        .output()
        .map_err(string)?;
    if !output.status.success() {
        return Err(
            "Rust stable toolchain is unavailable; run `rustup toolchain install stable`".into(),
        );
    }
    let rustc = String::from_utf8(output.stdout).map_err(string)?;
    let toolchain_bin = PathBuf::from(rustc.trim())
        .parent()
        .ok_or_else(|| "Rust stable toolchain path is invalid".to_owned())?
        .to_owned();
    let mut paths = vec![toolchain_bin.clone()];
    paths.extend(env::split_paths(&env::var_os("PATH").unwrap_or_default()));
    env::join_paths(paths).map_err(string)
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_package_name_only_from_package_section() {
        assert_eq!(
            cargo_package_name("[workspace]\nname = \"wrong\"\n[package]\nname = \"right\"\n"),
            Ok("right")
        );
    }

    #[test]
    fn rejects_unknown_commands() {
        assert_eq!(run_cli(&["pack".into()], Path::new(".")), Err(USAGE.into()));
    }
}
