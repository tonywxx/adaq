fn main() {
    if let Err(error) = adaq_component_tooling::run_cli(
        &std::env::args().skip(1).collect::<Vec<_>>(),
        &std::env::current_dir().unwrap_or_else(|error| {
            eprintln!("{error}");
            std::process::exit(1);
        }),
    ) {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
