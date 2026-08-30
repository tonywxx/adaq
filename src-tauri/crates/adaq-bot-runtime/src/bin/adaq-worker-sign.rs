use std::{env, fs};

use adaq_bot_runtime::{WorkerArtifactSignature, current_platform_tag};
use ed25519_dalek::{SigningKey, pkcs8::DecodePrivateKey};

fn main() {
    if let Err(error) = run() {
        eprintln!("adaq-worker-sign: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let args = env::args().skip(1).collect::<Vec<_>>();
    let artifact = flag(&args, "--artifact")?;
    let output_path = flag(&args, "--output")?;
    let platform = flag_optional(&args, "--platform")?.unwrap_or_else(current_platform_tag);
    let key_hex = env::var("ADAQ_WORKER_SIGNING_PRIVATE_KEY_HEX").or_else(|_| {
        let path = env::var("ADAQ_WORKER_SIGNING_PRIVATE_KEY_FILE")
            .map_err(|_| std::env::VarError::NotPresent)?;
        fs::read_to_string(path).map_err(|_| std::env::VarError::NotPresent)
    }).map_err(|_| "ADAQ_WORKER_SIGNING_PRIVATE_KEY_HEX or ADAQ_WORKER_SIGNING_PRIVATE_KEY_FILE is required".to_owned())?;
    let key = decode_key(key_hex.trim())?;
    let bytes = fs::read(&artifact).map_err(|_| "worker artifact could not be read".to_owned())?;
    let signature = WorkerArtifactSignature::sign(&bytes, platform, &key)?;
    let output = serde_json::to_vec_pretty(&signature).map_err(|error| error.to_string())?;
    fs::write(output_path, output)
        .map_err(|_| "worker signature could not be written".to_owned())?;
    Ok(())
}

fn flag(args: &[String], name: &str) -> Result<String, String> {
    let value = flag_optional(args, name)?;
    value.ok_or_else(|| format!("{} is required", name))
}

fn flag_optional(args: &[String], name: &str) -> Result<Option<String>, String> {
    let mut found = None;
    let mut values = args.iter();
    while let Some(arg) = values.next() {
        if arg == name {
            if found.is_some() {
                return Err(format!("{} was provided more than once", name));
            }
            found = Some(
                values
                    .next()
                    .ok_or_else(|| format!("{} requires a value", name))?
                    .clone(),
            );
        }
    }
    Ok(found)
}

fn decode_key(value: &str) -> Result<[u8; 32], String> {
    if value.starts_with("-----BEGIN") {
        return SigningKey::from_pkcs8_pem(value)
            .map(|key| key.to_bytes())
            .map_err(|_| "worker signing key PEM is invalid".to_owned());
    }
    if value.len() != 64 {
        return Err("worker signing key must be 32 bytes of hex".into());
    }
    let mut key = [0u8; 32];
    for (index, byte) in key.iter_mut().enumerate() {
        *byte = u8::from_str_radix(&value[index * 2..index * 2 + 2], 16)
            .map_err(|_| "worker signing key must be hexadecimal".to_owned())?;
    }
    Ok(key)
}
