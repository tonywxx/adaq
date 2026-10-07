fn main() {
    println!("cargo:rerun-if-env-changed=VITE_SUPABASE_URL");
    println!("cargo:rerun-if-env-changed=VITE_SUPABASE_PUBLISHABLE_KEY");
    println!("cargo:rerun-if-env-changed=VITE_SUPABASE_ANON_KEY");

    if let Ok(value) = std::env::var("VITE_SUPABASE_URL") {
        println!("cargo:rustc-env=ADAQ_SUPABASE_URL={value}");
    }
    if let Ok(value) = std::env::var("VITE_SUPABASE_PUBLISHABLE_KEY") {
        println!("cargo:rustc-env=ADAQ_SUPABASE_ANON_KEY={value}");
    } else if let Ok(value) = std::env::var("VITE_SUPABASE_ANON_KEY") {
        println!("cargo:rustc-env=ADAQ_SUPABASE_ANON_KEY={value}");
    }

    let mut attributes = tauri_build::Attributes::new();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Tauri embeds its manifest only in binaries; mock-runtime tests also need Common Controls v6.
        // https://github.com/tauri-apps/tauri/issues/13419
        attributes = attributes
            .windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!(
            "cargo:rustc-link-arg=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'"
        );
    }
    tauri_build::try_build(attributes).expect("failed to run tauri-build")
}
