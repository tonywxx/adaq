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

    tauri_build::build()
}
