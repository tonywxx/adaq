use adaq_bot_runtime::{
    ProtocolSequence, WORKER_ARTIFACT_NAME, WORKER_ARTIFACT_VERSION, WORKER_PROTOCOL_VERSION,
    WORKER_RUNTIME_VERSION, WorkerMessage, current_platform_tag, decode_frame, encode_frame,
    read_bounded_line, sha256_hex,
};
use std::{
    fs,
    io::{self, BufReader, BufWriter, Write},
    thread,
    time::Duration,
};

fn main() {
    if let Err(error) = run() {
        eprintln!("worker probe: {error}");
    }
}

fn run() -> Result<(), String> {
    let mode = std::env::current_exe()
        .map_err(|error| error.to_string())?
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or_default()
        .to_owned();
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let artifact_sha256 = sha256_hex(&fs::read(executable).map_err(|error| error.to_string())?);
    let platform = current_platform_tag();
    let mut output = BufWriter::new(io::stdout());
    let mut outbound = ProtocolSequence::default();
    let protocol_version = if mode.contains("handshake-failure") {
        "adaq-bot-worker-ipc@invalid"
    } else {
        WORKER_PROTOCOL_VERSION
    };
    let hello_sequence = outbound.next();
    write_message(
        &mut output,
        &WorkerMessage::Hello {
            sequence: hello_sequence,
            protocol_version: protocol_version.into(),
            artifact_name: WORKER_ARTIFACT_NAME.into(),
            artifact_version: WORKER_ARTIFACT_VERSION.into(),
            platform,
            runtime_version: WORKER_RUNTIME_VERSION.into(),
            artifact_sha256,
        },
        64 * 1024 * 1024,
    )?;
    if mode.contains("handshake-failure") {
        return Ok(());
    }

    let stdin = io::stdin();
    let mut input = BufReader::new(stdin.lock());
    let frame = read_bounded_line(&mut input, 64 * 1024 * 1024)
        .map_err(|error| error.to_string())?
        .ok_or_else(|| "initialize frame missing".to_owned())?;
    let initialize = decode_frame(&frame, 64 * 1024 * 1024).map_err(|error| error.to_string())?;
    let (request_id, bundle) = match initialize {
        WorkerMessage::Initialize {
            request_id, bundle, ..
        } => (request_id, bundle),
        _ => return Err("initialize message missing".into()),
    };
    let policy = bundle.input.worker_policy.clone();
    let initialized_sequence = outbound.next();
    write_message(
        &mut output,
        &WorkerMessage::Initialized {
            sequence: initialized_sequence,
            request_id,
            bundle_identity: bundle.identity,
            world: bundle.input.strategy.world,
        },
        policy.max_frame_bytes as usize,
    )?;

    if mode.contains("malformed") {
        output
            .write_all(b"not-json\n")
            .map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
        return Ok(());
    }
    if mode.contains("oversized") {
        let frame = vec![b'x'; policy.max_frame_bytes as usize + 1];
        output
            .write_all(&frame)
            .map_err(|error| error.to_string())?;
        output.flush().map_err(|error| error.to_string())?;
        return Ok(());
    }
    if mode.contains("crash") {
        return Ok(());
    }
    if mode.contains("hang") || mode.contains("missed-heartbeat") {
        loop {
            thread::sleep(Duration::from_secs(1));
        }
    }
    if mode.contains("late") {
        let _ = read_bounded_line(&mut input, policy.max_frame_bytes as usize);
        thread::sleep(Duration::from_millis(100));
        return Ok(());
    }

    let mut inbound = ProtocolSequence::default();
    loop {
        let Some(frame) = read_bounded_line(&mut input, policy.max_frame_bytes as usize)
            .map_err(|error| error.to_string())?
        else {
            return Ok(());
        };
        let message = decode_frame(&frame, policy.max_frame_bytes as usize)
            .map_err(|error| error.to_string())?;
        inbound
            .accept(message.sequence())
            .map_err(|error| error.to_string())?;
        if let WorkerMessage::Shutdown { request_id, .. } = message {
            let shutdown_sequence = outbound.next();
            write_message(
                &mut output,
                &WorkerMessage::ShutdownAck {
                    sequence: shutdown_sequence,
                    request_id,
                },
                policy.max_frame_bytes as usize,
            )?;
            return Ok(());
        }
    }
}

fn write_message(
    output: &mut BufWriter<impl Write>,
    message: &WorkerMessage,
    max_frame_bytes: usize,
) -> Result<(), String> {
    output
        .write_all(&encode_frame(message, max_frame_bytes).map_err(|error| error.to_string())?)
        .map_err(|error| error.to_string())?;
    output.flush().map_err(|error| error.to_string())
}
