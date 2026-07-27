use std::{env, fs, path::Path};

use ada_backtest_core::{ComponentManifest, ComponentPackage, pack_component};

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let arguments = env::args().skip(1).collect::<Vec<_>>();
    match arguments.as_slice() {
        [command, manifest, wasm, output] if command == "pack" => {
            let manifest: ComponentManifest = serde_json::from_slice(
                &fs::read(manifest).map_err(string)?,
            ).map_err(string)?;
            let package = pack_component(manifest, &fs::read(wasm).map_err(string)?)
                .map_err(string)?;
            if Path::new(output).extension().and_then(|value| value.to_str()) != Some("adaq") {
                return Err("Output must use the .adaq extension".into());
            }
            fs::write(output, package).map_err(string)
        }
        [command, package] if command == "verify" => {
            let package = ComponentPackage::read(&fs::read(package).map_err(string)?)
                .map_err(string)?;
            println!(
                "verified {} {} ({})",
                package.manifest.name, package.manifest.version, package.archive_sha256
            );
            Ok(())
        }
        _ => Err(
            "usage: adaq-component pack <manifest.json> <component.wasm> <output.adaq>\n       adaq-component verify <package.adaq>"
                .into(),
        ),
    }
}

fn string(error: impl std::fmt::Display) -> String {
    error.to_string()
}
