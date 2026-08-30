fn main() {
    if let Err(error) = adaq_bot_worker::run() {
        eprintln!("adaq-bot-worker: {error}");
        std::process::exit(1);
    }
}
