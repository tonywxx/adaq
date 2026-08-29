//! Private Host/Runner protocol and retained Attempt boundary.
//!
//! The module contains data contracts only. Process creation and platform
//! termination stay in the Tauri control plane so this crate never gains
//! credentials, database paths, or application state.

use crate::{
    HostResourcePolicy, PythonResearchError, TrustDecision, invalid, is_sha256, sha256,
    validate_user_id,
};
use arrow_array::{Array, Float32Array, Float64Array, StringArray};
use arrow_ipc::reader::StreamReader;
use arrow_schema::DataType;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{Cursor, Read, Write},
    net::{TcpListener, TcpStream},
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const RUNNER_PROTOCOL_VERSION: &str = "adaq-python-runner@1";
pub const MAX_CONTROL_MESSAGE_BYTES: usize = 16 * 1024 * 1024;
pub const MAX_LOG_BYTES: usize = 1024 * 1024;
pub const MAX_RESULT_ROWS: usize = 10_000_000;
const MANAGED_RUNNER_BOOTSTRAP: &str = "import runpy,sys; sys.path.insert(0,sys.argv.pop(1)); sys.argv[0]='adaq_runner'; runpy.run_module('adaq_runner.__main__',run_name='__main__')";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Handshake {
    pub protocol: String,
    pub sdk_artifact_sha256: String,
    pub revision_sha256: String,
    pub environment_sha256: String,
    pub attempt_id: String,
    pub loopback: bool,
    pub one_time_token: String,
}

impl Handshake {
    pub fn validate(
        &self,
        sdk_artifact_sha256: &str,
        revision_sha256: &str,
        environment_sha256: &str,
        attempt_id: &str,
        expected_token: &str,
    ) -> Result<(), PythonResearchError> {
        if self.protocol != RUNNER_PROTOCOL_VERSION
            || self.sdk_artifact_sha256 != sdk_artifact_sha256
            || self.revision_sha256 != revision_sha256
            || self.environment_sha256 != environment_sha256
            || self.attempt_id != attempt_id
            || !self.loopback
            || self.one_time_token != expected_token
            || self.one_time_token.len() < 32
            || !is_sha256(&self.sdk_artifact_sha256)
            || !is_sha256(&self.revision_sha256)
            || !is_sha256(&self.environment_sha256)
        {
            return Err(invalid("runner-handshake-rejected"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "kebab-case",
    rename_all_fields = "camelCase"
)]
pub enum ControlMessage {
    Hello { handshake: Handshake },
    Ready,
    Execute { execution: AttemptExecution },
    Progress { completed: u64, total: u64 },
    Diagnostic { code: String, message: String },
    Artifact { artifact: StagedArtifact },
    Result { result: StagedResult },
    ConformanceResult { result: ConformanceResult },
    Cancel,
    Shutdown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConformanceResult {
    pub attempt_id: String,
    pub project_id: String,
    pub project_kind: String,
    pub entry_point: String,
    #[serde(default)]
    pub payload: Option<serde_json::Value>,
}

#[derive(Debug, Clone)]
pub struct RunnerLaunchSpec {
    pub python_executable: PathBuf,
    pub runner_script: PathBuf,
    pub runner_wheel: Option<PathBuf>,
    pub project_root: PathBuf,
    pub entry_point: String,
    pub sdk_wheel: Option<PathBuf>,
    pub adapter_wheel: Option<PathBuf>,
    pub handshake: Handshake,
    pub environment: PrivateChildEnvironment,
    pub staging_root: PathBuf,
    pub staged_result_path: PathBuf,
    pub staged_result_relative_path: String,
    pub execution: AttemptExecution,
    pub seed: u64,
    pub max_wall_ms: u64,
    pub max_memory_bytes: u64,
    pub max_processes: u32,
    pub max_control_bytes: usize,
    pub max_arrow_bytes: usize,
    pub max_staged_bytes: usize,
    pub max_artifact_bytes: usize,
    pub max_log_bytes: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RunnerExecution {
    pub conformance: Option<ConformanceResult>,
    pub staged_result: Option<StagedResult>,
    pub staged_artifact: Option<StagedArtifact>,
    pub log: Vec<u8>,
    pub log_truncated: bool,
}

pub fn run_process(
    spec: &RunnerLaunchSpec,
    cancelled: impl Fn() -> bool,
) -> Result<RunnerExecution, PythonResearchError> {
    if spec.max_wall_ms == 0
        || spec.max_memory_bytes == 0
        || spec.max_processes == 0
        || spec.max_control_bytes == 0
        || spec.max_control_bytes > MAX_CONTROL_MESSAGE_BYTES
        || spec.max_arrow_bytes == 0
        || spec.max_staged_bytes == 0
        || spec.max_artifact_bytes == 0
        || spec.max_artifact_bytes > spec.max_staged_bytes
        || spec.max_log_bytes == 0
    {
        return Err(invalid("runner-process-policy-invalid"));
    }
    spec.handshake.validate(
        &spec.handshake.sdk_artifact_sha256,
        &spec.handshake.revision_sha256,
        &spec.handshake.environment_sha256,
        &spec.handshake.attempt_id,
        &spec.handshake.one_time_token,
    )?;
    let listener = TcpListener::bind("127.0.0.1:0")
        .map_err(|error| invalid(format!("runner-loopback-bind-failed:{error}")))?;
    listener
        .set_nonblocking(true)
        .map_err(|error| invalid(format!("runner-loopback-configure-failed:{error}")))?;
    let address = listener
        .local_addr()
        .map_err(|error| invalid(format!("runner-loopback-address-failed:{error}")))?;
    let mut command = Command::new(&spec.python_executable);
    command.arg("-I").arg("-B");
    if let Some(runner_wheel) = &spec.runner_wheel {
        command
            .arg("-c")
            .arg(MANAGED_RUNNER_BOOTSTRAP)
            .arg(runner_wheel);
    } else {
        command.arg(&spec.runner_script);
    }
    command
        .arg("--connect")
        .arg(address.to_string())
        .arg("--project-root")
        .arg(&spec.project_root)
        .arg("--entry-point")
        .arg(&spec.entry_point)
        .arg("--result-path")
        .arg(&spec.staged_result_path)
        .arg("--result-name")
        .arg(&spec.staged_result_relative_path)
        .env_clear()
        .env(
            "ADAQ_EXPECTED_SDK_SHA256",
            &spec.handshake.sdk_artifact_sha256,
        )
        .env(
            "ADAQ_EXPECTED_REVISION_SHA256",
            &spec.handshake.revision_sha256,
        )
        .env(
            "ADAQ_EXPECTED_ENVIRONMENT_SHA256",
            &spec.handshake.environment_sha256,
        )
        .env("ADAQ_EXPECTED_ATTEMPT_ID", &spec.handshake.attempt_id)
        .env("ADAQ_RUNNER_TOKEN", &spec.handshake.one_time_token)
        .env("ADAQ_ATTEMPT_SEED", spec.seed.to_string())
        .env("ADAQ_MAX_MEMORY_BYTES", spec.max_memory_bytes.to_string())
        .env(
            "ADAQ_MAX_ARTIFACT_BYTES",
            spec.max_artifact_bytes.to_string(),
        )
        .env("ADAQ_MAX_PROCESSES", spec.max_processes.to_string())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(windows)]
    if let Some(system_root) = std::env::var_os("SystemRoot") {
        // Winsock provider loading requires the system root after env_clear.
        command.env("SystemRoot", system_root);
    }
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    if let Some(sdk_wheel) = &spec.sdk_wheel {
        command.arg("--sdk-wheel").arg(sdk_wheel);
    }
    if let Some(adapter_wheel) = &spec.adapter_wheel {
        command.arg("--adapter-wheel").arg(adapter_wheel);
    }
    for (key, value) in spec.environment.variables() {
        command.env(key, value);
    }
    let mut child = command
        .spawn()
        .map_err(|error| invalid(format!("runner-process-spawn-failed:{error}")))?;
    #[cfg(windows)]
    let _windows_job =
        match configure_windows_job(&child, spec.max_memory_bytes, spec.max_processes) {
            Ok(job) => job,
            Err(error) => {
                terminate_child(&mut child);
                return Err(error);
            }
        };
    let log = Arc::new(Mutex::new(BoundedLog::new(spec.max_log_bytes)?));
    let mut log_readers = Vec::new();
    if let Some(stdout) = child.stdout.take() {
        log_readers.push(spawn_log_reader(stdout, log.clone()));
    }
    if let Some(stderr) = child.stderr.take() {
        log_readers.push(spawn_log_reader(stderr, log.clone()));
    }
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(spec.max_wall_ms))
        .unwrap_or_else(Instant::now);
    let mut stream = match accept_runner(&listener, &mut child, deadline, &cancelled) {
        Ok(stream) => stream,
        Err(error) => {
            terminate_child(&mut child);
            join_log_readers(log_readers);
            let diagnostic = log
                .lock()
                .map(|value| String::from_utf8_lossy(value.as_bytes()).into_owned())
                .unwrap_or_default();
            return Err(if diagnostic.is_empty() {
                error
            } else {
                invalid(format!("{}:{diagnostic}", error.0))
            });
        }
    };
    if let Err(error) = stream.set_nonblocking(true) {
        terminate_child(&mut child);
        join_log_readers(log_readers);
        return Err(invalid(format!("runner-stream-configure-failed:{error}")));
    }
    let hello = match read_control_poll(&mut stream, deadline, &|| false, spec.max_control_bytes) {
        Ok(message) => message,
        Err(error) => {
            terminate_child(&mut child);
            join_log_readers(log_readers);
            return Err(error);
        }
    };
    let ControlMessage::Hello { handshake } = hello else {
        terminate_child(&mut child);
        join_log_readers(log_readers);
        return Err(invalid("runner-handshake-required"));
    };
    if let Err(error) = handshake.validate(
        &spec.handshake.sdk_artifact_sha256,
        &spec.handshake.revision_sha256,
        &spec.handshake.environment_sha256,
        &spec.handshake.attempt_id,
        &spec.handshake.one_time_token,
    ) {
        terminate_child(&mut child);
        join_log_readers(log_readers);
        return Err(error);
    }
    if let Err(error) = write_control_poll(
        &mut stream,
        &ControlMessage::Execute {
            execution: spec.execution.clone(),
        },
        deadline,
    ) {
        terminate_child(&mut child);
        join_log_readers(log_readers);
        return Err(error);
    }
    let mut result = loop {
        if cancelled() {
            cancel_child(&mut child, &mut stream, deadline);
            break Err(invalid("runner-cancelled"));
        }
        if Instant::now() >= deadline {
            terminate_child(&mut child);
            break Err(invalid("runner-deadline-exceeded"));
        }
        match read_control_poll(&mut stream, deadline, &cancelled, spec.max_control_bytes) {
            Ok(ControlMessage::Diagnostic { code, message }) => {
                if let Ok(mut output) = log.lock() {
                    output.append(&format!("{code}: {message}"));
                }
            }
            Ok(ControlMessage::Artifact { artifact }) => {
                let artifact_limit = if artifact.media_type == "application/vnd.apache.arrow.stream"
                {
                    spec.max_arrow_bytes.min(spec.max_artifact_bytes)
                } else {
                    spec.max_artifact_bytes
                };
                let bytes = match read_staged_artifact(
                    &spec.staging_root,
                    &artifact,
                    &spec.handshake.attempt_id,
                    artifact_limit,
                ) {
                    Ok(bytes) => bytes,
                    Err(error) => {
                        terminate_child(&mut child);
                        break Err(error);
                    }
                };
                let validated = (|| -> Result<Option<ConformanceResult>, PythonResearchError> {
                    match artifact.media_type.as_str() {
                        "application/json" => Ok(Some(
                            serde_json::from_value(parse_canonical_json(&bytes)?).map_err(
                                |error| invalid(format!("runner-result-invalid:{error}")),
                            )?,
                        )),
                        "application/vnd.apache.arrow.stream" => {
                            let descriptor = ArrowTableDescriptor {
                                columns: artifact
                                    .columns
                                    .clone()
                                    .ok_or_else(|| invalid("runner-arrow-descriptor-missing"))?,
                                row_count: artifact
                                    .row_count
                                    .ok_or_else(|| invalid("runner-arrow-descriptor-missing"))?,
                                artifact: artifact.clone(),
                            };
                            validate_arrow_ipc(
                                &spec.staging_root,
                                &descriptor,
                                &spec.handshake.attempt_id,
                                artifact_limit,
                            )?;
                            Ok(None)
                        }
                        _ => Err(invalid("runner-artifact-media-type-invalid")),
                    }
                })();
                let conformance = match validated {
                    Ok(conformance) => conformance,
                    Err(error) => {
                        terminate_child(&mut child);
                        break Err(error);
                    }
                };
                let _ = write_control_poll(&mut stream, &ControlMessage::Shutdown, deadline);
                break Ok(RunnerExecution {
                    conformance,
                    staged_result: None,
                    staged_artifact: Some(artifact),
                    log: Vec::new(),
                    log_truncated: false,
                });
            }
            Ok(ControlMessage::ConformanceResult { .. } | ControlMessage::Result { .. }) => {
                terminate_child(&mut child);
                break Err(invalid("runner-inline-result-rejected"));
            }
            Ok(ControlMessage::Progress { .. } | ControlMessage::Ready) => {}
            Ok(
                ControlMessage::Hello { .. }
                | ControlMessage::Execute { .. }
                | ControlMessage::Cancel
                | ControlMessage::Shutdown,
            ) => {
                terminate_child(&mut child);
                break Err(invalid("runner-control-message-unexpected"));
            }
            Err(error) if error.0 == "runner-cancelled" => {
                cancel_child(&mut child, &mut stream, deadline);
                break Err(error);
            }
            Err(error) => {
                terminate_child(&mut child);
                break Err(error);
            }
        }
        if child
            .try_wait()
            .map_err(|error| invalid(format!("runner-process-wait-failed:{error}")))?
            .is_some()
        {
            break Err(invalid("runner-process-exited-without-result"));
        }
    };
    if !wait_child_until(&mut child, deadline) {
        terminate_child(&mut child);
        result = Err(invalid("runner-process-did-not-exit"));
    }
    join_log_readers(log_readers);
    let (log, log_truncated) = log
        .lock()
        .map(|value| (value.as_bytes().to_vec(), value.truncated()))
        .unwrap_or_default();
    match result {
        Ok(mut value) => {
            value.log = log;
            value.log_truncated = log_truncated;
            Ok(value)
        }
        Err(error) if log.is_empty() => Err(error),
        Err(error) => Err(invalid(format!(
            "{}:{}",
            error.0,
            String::from_utf8_lossy(&log)
        ))),
    }
}

fn spawn_log_reader<R>(mut reader: R, log: Arc<Mutex<BoundedLog>>) -> thread::JoinHandle<()>
where
    R: Read + Send + 'static,
{
    thread::spawn(move || {
        let mut buffer = [0u8; 4096];
        loop {
            match reader.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(size) => {
                    let text = String::from_utf8_lossy(&buffer[..size]);
                    if let Ok(mut log) = log.lock() {
                        log.append(&text);
                    }
                }
            }
        }
    })
}

fn join_log_readers(readers: Vec<thread::JoinHandle<()>>) {
    for reader in readers {
        let _ = reader.join();
    }
}

fn parse_canonical_json(bytes: &[u8]) -> Result<serde_json::Value, PythonResearchError> {
    let value: serde_json::Value = serde_json::from_slice(bytes)
        .map_err(|error| invalid(format!("runner-result-json-invalid:{error}")))?;
    let canonical = serde_json::to_vec(&value).map_err(|error| invalid(error.to_string()))?;
    if canonical != bytes {
        return Err(invalid("runner-result-json-not-canonical"));
    }
    Ok(value)
}

fn accept_runner(
    listener: &TcpListener,
    child: &mut std::process::Child,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
) -> Result<TcpStream, PythonResearchError> {
    loop {
        if cancelled() {
            terminate_child(child);
            return Err(invalid("runner-cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(invalid("runner-handshake-timeout"));
        }
        match listener.accept() {
            Ok((stream, _)) => return Ok(stream),
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(5));
            }
            Err(error) => return Err(invalid(format!("runner-loopback-accept-failed:{error}"))),
        }
        if child
            .try_wait()
            .map_err(|error| invalid(format!("runner-process-wait-failed:{error}")))?
            .is_some()
        {
            return Err(invalid("runner-process-exited-before-handshake"));
        }
    }
}

fn read_control_poll(
    stream: &mut TcpStream,
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
    max_control_bytes: usize,
) -> Result<ControlMessage, PythonResearchError> {
    let mut header = [0u8; 4];
    read_exact_poll(stream, &mut header, deadline, cancelled)?;
    let length = u32::from_be_bytes(header) as usize;
    if length > max_control_bytes {
        return Err(invalid("runner-control-message-too-large"));
    }
    let mut body = vec![0u8; length];
    read_exact_poll(stream, &mut body, deadline, cancelled)?;
    decode_control(&[header.as_slice(), body.as_slice()].concat())
}

fn read_exact_poll(
    stream: &mut TcpStream,
    buffer: &mut [u8],
    deadline: Instant,
    cancelled: &impl Fn() -> bool,
) -> Result<(), PythonResearchError> {
    let mut offset = 0;
    while offset < buffer.len() {
        if cancelled() {
            return Err(invalid("runner-cancelled"));
        }
        if Instant::now() >= deadline {
            return Err(invalid("runner-deadline-exceeded"));
        }
        match stream.read(&mut buffer[offset..]) {
            Ok(0) => return Err(invalid("runner-control-stream-closed")),
            Ok(size) => offset += size,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(invalid(format!("runner-control-read-failed:{error}"))),
        }
    }
    Ok(())
}

fn write_control_poll(
    stream: &mut TcpStream,
    message: &ControlMessage,
    deadline: Instant,
) -> Result<(), PythonResearchError> {
    let frame = encode_control(message)?;
    let mut offset = 0;
    while offset < frame.len() {
        if Instant::now() >= deadline {
            return Err(invalid("runner-deadline-exceeded"));
        }
        match stream.write(&frame[offset..]) {
            Ok(0) => return Err(invalid("runner-control-stream-closed")),
            Ok(size) => offset += size,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(2));
            }
            Err(error) => return Err(invalid(format!("runner-control-write-failed:{error}"))),
        }
    }
    Ok(())
}

#[cfg(windows)]
struct WindowsJobObject {
    handle: windows_sys::Win32::Foundation::HANDLE,
}

#[cfg(windows)]
impl Drop for WindowsJobObject {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.handle);
        }
    }
}

#[cfg(windows)]
fn configure_windows_job(
    child: &std::process::Child,
    max_memory_bytes: u64,
    max_processes: u32,
) -> Result<WindowsJobObject, PythonResearchError> {
    use std::{mem::size_of, os::windows::io::AsRawHandle, ptr::null};
    use windows_sys::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
        JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JobObjectExtendedLimitInformation,
        SetInformationJobObject,
    };

    let process_memory_limit = usize::try_from(max_memory_bytes)
        .map_err(|_| invalid("runner-windows-memory-limit-too-large"))?;
    let handle = unsafe { CreateJobObjectW(null(), null()) };
    if handle.is_null() {
        return Err(invalid(format!(
            "runner-windows-job-create-failed:{}",
            std::io::Error::last_os_error()
        )));
    }
    let job = WindowsJobObject { handle };
    let mut limits = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
    limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_ACTIVE_PROCESS
        | JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE
        | JOB_OBJECT_LIMIT_PROCESS_MEMORY;
    limits.BasicLimitInformation.ActiveProcessLimit = max_processes;
    limits.ProcessMemoryLimit = process_memory_limit;
    let set = unsafe {
        SetInformationJobObject(
            job.handle,
            JobObjectExtendedLimitInformation,
            (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
            size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        )
    };
    if set == 0 {
        return Err(invalid(format!(
            "runner-windows-job-configure-failed:{}",
            std::io::Error::last_os_error()
        )));
    }
    let assigned = unsafe { AssignProcessToJobObject(job.handle, child.as_raw_handle()) };
    if assigned == 0 {
        return Err(invalid(format!(
            "runner-windows-job-assign-failed:{}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(job)
}

fn terminate_child(child: &mut std::process::Child) {
    #[cfg(unix)]
    if let Ok(pid) = i32::try_from(child.id()) {
        // The Runner owns this process group; terminate descendants before
        // waiting so a non-cooperative child cannot outlive the Attempt.
        unsafe {
            libc::kill(-pid, libc::SIGKILL);
        }
    }
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/PID", &child.id().to_string(), "/T", "/F"])
            .status();
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn cancel_child(child: &mut std::process::Child, stream: &mut TcpStream, deadline: Instant) {
    let _ = write_control_poll(stream, &ControlMessage::Cancel, deadline);
    let grace_deadline = Instant::now()
        .checked_add(Duration::from_millis(250))
        .unwrap_or(deadline)
        .min(deadline);
    while Instant::now() < grace_deadline {
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        thread::sleep(Duration::from_millis(5));
    }
    if child.try_wait().ok().flatten().is_none() {
        terminate_child(child);
    }
}

fn wait_child_until(child: &mut std::process::Child, deadline: Instant) -> bool {
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return true,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) | Err(_) => return false,
        }
    }
}

pub fn encode_control(message: &ControlMessage) -> Result<Vec<u8>, PythonResearchError> {
    let body = serde_json::to_vec(message).map_err(|error| invalid(error.to_string()))?;
    if body.len() > MAX_CONTROL_MESSAGE_BYTES || body.len() > u32::MAX as usize {
        return Err(invalid("runner-control-message-too-large"));
    }
    let mut frame = Vec::with_capacity(body.len() + 4);
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

pub fn decode_control(frame: &[u8]) -> Result<ControlMessage, PythonResearchError> {
    if frame.len() < 4 {
        return Err(invalid("runner-control-frame-truncated"));
    }
    let length = u32::from_be_bytes(frame[..4].try_into().unwrap()) as usize;
    if length > MAX_CONTROL_MESSAGE_BYTES || frame.len() != length + 4 {
        return Err(invalid("runner-control-frame-length-invalid"));
    }
    serde_json::from_slice(&frame[4..])
        .map_err(|error| invalid(format!("runner-control-json-invalid:{error}")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum AttemptStatus {
    Pending,
    Running,
    Completed,
    Failed,
    Cancelled,
    Interrupted,
    Stale,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct AttemptExecution {
    #[serde(default)]
    pub sdk_artifact_sha256: String,
    #[serde(default)]
    pub runtime_artifact_sha256: String,
    #[serde(default)]
    pub entry_point: String,
    #[serde(default)]
    pub input_bindings: BTreeMap<String, String>,
    #[serde(default)]
    pub parameters: BTreeMap<String, String>,
    #[serde(default)]
    pub seed: u64,
    #[serde(default)]
    pub output_names: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<serde_json::Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResearchAttempt {
    pub attempt_id: String,
    pub user_id: String,
    #[serde(default)]
    pub project_id: String,
    pub revision_sha256: String,
    pub environment_sha256: String,
    pub queue_sequence: u64,
    pub status: AttemptStatus,
    pub source_attempt_id: Option<String>,
    pub failure_code: Option<String>,
    pub diagnostic: Option<String>,
    #[serde(default)]
    pub log: Option<String>,
    pub resource_policy: HostResourcePolicy,
    #[serde(default)]
    pub execution: AttemptExecution,
    #[serde(default)]
    pub progress_completed: u64,
    #[serde(default)]
    pub progress_total: u64,
    pub staged_result_sha256: Option<String>,
    #[serde(default)]
    pub cancel_requested: bool,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
}

impl ResearchAttempt {
    pub fn new(
        user_id: impl Into<String>,
        project_id: impl Into<String>,
        revision_sha256: impl Into<String>,
        environment_sha256: impl Into<String>,
        queue_sequence: u64,
        resource_policy: HostResourcePolicy,
    ) -> Result<Self, PythonResearchError> {
        let revision_sha256 = revision_sha256.into();
        let environment_sha256 = environment_sha256.into();
        if !is_sha256(&revision_sha256) || !is_sha256(&environment_sha256) {
            return Err(invalid("research-attempt-identity-invalid"));
        }
        resource_policy.validate()?;
        let now = now_ms();
        Ok(Self {
            attempt_id: sha256(
                format!("{}:{}:{}", now, queue_sequence, revision_sha256).as_bytes(),
            ),
            user_id: user_id.into(),
            project_id: project_id.into(),
            revision_sha256,
            environment_sha256,
            queue_sequence,
            status: AttemptStatus::Pending,
            source_attempt_id: None,
            failure_code: None,
            diagnostic: None,
            log: None,
            resource_policy,
            execution: AttemptExecution::default(),
            progress_completed: 0,
            progress_total: 1,
            staged_result_sha256: None,
            cancel_requested: false,
            created_at_ms: now,
            updated_at_ms: now,
        })
    }

    pub fn begin(&mut self) -> Result<(), PythonResearchError> {
        if self.status != AttemptStatus::Pending {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.status = AttemptStatus::Running;
        self.progress_completed = 0;
        self.progress_total = 1;
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn complete(&mut self, result_sha256: String) -> Result<(), PythonResearchError> {
        if self.status != AttemptStatus::Running
            || self.cancel_requested
            || !is_sha256(&result_sha256)
        {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.status = AttemptStatus::Completed;
        self.progress_completed = self.progress_total.max(1);
        self.progress_total = self.progress_completed;
        self.staged_result_sha256 = Some(result_sha256);
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn fail(
        &mut self,
        code: impl Into<String>,
        diagnostic: impl Into<String>,
    ) -> Result<(), PythonResearchError> {
        if !matches!(self.status, AttemptStatus::Pending | AttemptStatus::Running) {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.status = AttemptStatus::Failed;
        self.failure_code = Some(code.into());
        self.diagnostic = Some(bounded_diagnostic(&diagnostic.into()));
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn interrupt(
        &mut self,
        code: impl Into<String>,
        diagnostic: impl Into<String>,
    ) -> Result<(), PythonResearchError> {
        if self.status != AttemptStatus::Running {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.status = AttemptStatus::Interrupted;
        self.failure_code = Some(code.into());
        self.diagnostic = Some(bounded_diagnostic(&diagnostic.into()));
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn stale(&mut self, diagnostic: impl Into<String>) -> Result<(), PythonResearchError> {
        if !matches!(self.status, AttemptStatus::Pending | AttemptStatus::Running) {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.status = AttemptStatus::Stale;
        self.failure_code = Some("research-stale".into());
        self.diagnostic = Some(bounded_diagnostic(&diagnostic.into()));
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn record_host_failure(
        &mut self,
        code: impl Into<String>,
        diagnostic: impl Into<String>,
    ) -> Result<(), PythonResearchError> {
        if self.status != AttemptStatus::Completed
            || self.staged_result_sha256.is_none()
            || self.failure_code.is_some()
        {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.failure_code = Some(code.into());
        self.diagnostic = Some(bounded_diagnostic(&diagnostic.into()));
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn record_log(&mut self, value: impl Into<String>) -> Result<(), PythonResearchError> {
        if !matches!(
            self.status,
            AttemptStatus::Running | AttemptStatus::Completed
        ) {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.log = Some(bounded_diagnostic(&value.into()));
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn record_progress(
        &mut self,
        completed: u64,
        total: u64,
    ) -> Result<(), PythonResearchError> {
        if self.status != AttemptStatus::Running || total == 0 || completed > total {
            return Err(invalid("research-attempt-progress-invalid"));
        }
        self.progress_completed = completed;
        self.progress_total = total;
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn cancel(&mut self) -> Result<(), PythonResearchError> {
        if !matches!(self.status, AttemptStatus::Pending | AttemptStatus::Running) {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        if self.status == AttemptStatus::Pending {
            self.status = AttemptStatus::Cancelled;
        } else {
            self.cancel_requested = true;
        }
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn finish_cancel(&mut self) -> Result<(), PythonResearchError> {
        if self.status != AttemptStatus::Running || !self.cancel_requested {
            return Err(invalid("research-attempt-transition-invalid"));
        }
        self.status = AttemptStatus::Cancelled;
        self.updated_at_ms = now_ms();
        Ok(())
    }

    pub fn retry(&self, queue_sequence: u64) -> Result<Self, PythonResearchError> {
        if !matches!(
            self.status,
            AttemptStatus::Failed
                | AttemptStatus::Cancelled
                | AttemptStatus::Interrupted
                | AttemptStatus::Stale
        ) && !(self.status == AttemptStatus::Completed && self.failure_code.is_some())
        {
            return Err(invalid("research-attempt-retry-invalid"));
        }
        let mut next = Self::new(
            self.user_id.clone(),
            self.project_id.clone(),
            self.revision_sha256.clone(),
            self.environment_sha256.clone(),
            queue_sequence,
            self.resource_policy.clone(),
        )?;
        next.source_attempt_id = Some(self.attempt_id.clone());
        next.execution = self.execution.clone();
        Ok(next)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct AttemptDatabase {
    next_sequence: u64,
    attempts: BTreeMap<String, ResearchAttempt>,
}

/// Small persistent FIFO used by the native control-plane seam. The full
/// Feature/Factor runner remains the device-wide owner; this store supplies
/// Python's retained Attempt records to that owner.
#[derive(Clone)]
pub struct AttemptStore {
    path: PathBuf,
    database: Arc<Mutex<AttemptDatabase>>,
}

impl AttemptStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PythonResearchError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let database = if path.is_file() {
            serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| invalid(format!("research-attempt-store-invalid:{error}")))?
        } else {
            AttemptDatabase::default()
        };
        let store = Self {
            path,
            database: Arc::new(Mutex::new(database)),
        };
        store.recover_running()?;
        Ok(store)
    }

    pub fn enqueue(
        &self,
        user_id: impl Into<String>,
        project_id: impl Into<String>,
        revision_sha256: impl Into<String>,
        environment_sha256: impl Into<String>,
        policy: HostResourcePolicy,
    ) -> Result<ResearchAttempt, PythonResearchError> {
        self.enqueue_with_execution(
            user_id,
            project_id,
            revision_sha256,
            environment_sha256,
            policy,
            AttemptExecution::default(),
        )
    }

    pub fn enqueue_with_execution(
        &self,
        user_id: impl Into<String>,
        project_id: impl Into<String>,
        revision_sha256: impl Into<String>,
        environment_sha256: impl Into<String>,
        policy: HostResourcePolicy,
        execution: AttemptExecution,
    ) -> Result<ResearchAttempt, PythonResearchError> {
        let user_id = user_id.into();
        let project_id = project_id.into();
        let revision_sha256 = revision_sha256.into();
        let environment_sha256 = environment_sha256.into();
        let policy = HostResourcePolicy::m12_default().lowered_by(&policy)?;
        let mut database = self.lock()?;
        if let Some(existing) = database.attempts.values().find(|attempt| {
            attempt.user_id == user_id
                && attempt.project_id == project_id
                && attempt.revision_sha256 == revision_sha256
                && attempt.environment_sha256 == environment_sha256
                && attempt.execution == execution
                && attempt.resource_policy == policy
                && matches!(
                    attempt.status,
                    AttemptStatus::Pending | AttemptStatus::Running
                )
        }) {
            return Ok(existing.clone());
        }
        database.next_sequence = database.next_sequence.saturating_add(1);
        let attempt = ResearchAttempt::new(
            user_id,
            project_id,
            revision_sha256,
            environment_sha256,
            database.next_sequence,
            policy,
        )?;
        let mut attempt = attempt;
        attempt.execution = execution;
        database
            .attempts
            .insert(attempt.attempt_id.clone(), attempt.clone());
        self.persist_locked(&database)?;
        Ok(attempt)
    }

    pub fn list(&self, user_id: &str) -> Result<Vec<ResearchAttempt>, PythonResearchError> {
        let database = self.lock()?;
        let mut attempts = database
            .attempts
            .values()
            .filter(|attempt| attempt.user_id == user_id)
            .cloned()
            .collect::<Vec<_>>();
        attempts.sort_by_key(|attempt| attempt.queue_sequence);
        Ok(attempts)
    }

    pub fn cancel_user(&self, user_id: &str) -> Result<(), PythonResearchError> {
        validate_user_id(user_id)?;
        let mut database = self.lock()?;
        let mut changed = false;
        for attempt in database.attempts.values_mut().filter(|attempt| {
            attempt.user_id == user_id
                && matches!(
                    attempt.status,
                    AttemptStatus::Pending | AttemptStatus::Running
                )
        }) {
            attempt.cancel()?;
            changed = true;
        }
        if changed {
            self.persist_locked(&database)?;
        }
        Ok(())
    }

    pub fn has_active_for_user(&self, user_id: &str) -> Result<bool, PythonResearchError> {
        validate_user_id(user_id)?;
        Ok(self.lock()?.attempts.values().any(|attempt| {
            attempt.user_id == user_id
                && matches!(
                    attempt.status,
                    AttemptStatus::Pending | AttemptStatus::Running
                )
        }))
    }

    pub fn get(&self, attempt_id: &str) -> Result<ResearchAttempt, PythonResearchError> {
        self.lock()?
            .attempts
            .get(attempt_id)
            .cloned()
            .ok_or_else(|| invalid("research-attempt-not-found"))
    }

    pub fn next_pending(&self) -> Result<Option<ResearchAttempt>, PythonResearchError> {
        let database = self.lock()?;
        Ok(database
            .attempts
            .values()
            .filter(|attempt| attempt.status == AttemptStatus::Pending)
            .min_by_key(|attempt| attempt.queue_sequence)
            .cloned())
    }

    pub fn next_runnable(&self) -> Result<Option<ResearchAttempt>, PythonResearchError> {
        let database = self.lock()?;
        Ok(database
            .attempts
            .values()
            .filter(|attempt| attempt.status == AttemptStatus::Pending)
            .min_by_key(|attempt| attempt.queue_sequence)
            .cloned())
    }

    pub fn pending_attempts(&self) -> Result<Vec<ResearchAttempt>, PythonResearchError> {
        let database = self.lock()?;
        let mut attempts = database
            .attempts
            .values()
            .filter(|attempt| attempt.status == AttemptStatus::Pending)
            .cloned()
            .collect::<Vec<_>>();
        attempts.sort_by_key(|attempt| attempt.queue_sequence);
        Ok(attempts)
    }

    pub fn active_environment_ids(&self) -> Result<BTreeSet<String>, PythonResearchError> {
        Ok(self
            .lock()?
            .attempts
            .values()
            .filter(|attempt| {
                matches!(
                    attempt.status,
                    AttemptStatus::Pending | AttemptStatus::Running
                )
            })
            .map(|attempt| attempt.environment_sha256.clone())
            .collect())
    }

    pub fn reset_user(&self, user_id: &str) -> Result<(), PythonResearchError> {
        validate_user_id(user_id)?;
        let mut database = self.lock()?;
        database
            .attempts
            .retain(|_, attempt| attempt.user_id != user_id);
        self.persist_locked(&database)
    }

    pub fn transition(
        &self,
        attempt_id: &str,
        transition: AttemptTransition,
    ) -> Result<ResearchAttempt, PythonResearchError> {
        let mut database = self.lock()?;
        let attempt = database
            .attempts
            .get_mut(attempt_id)
            .ok_or_else(|| invalid("research-attempt-not-found"))?;
        match transition {
            AttemptTransition::Begin => attempt.begin()?,
            AttemptTransition::Complete { result_sha256 } => attempt.complete(result_sha256)?,
            AttemptTransition::Fail { code, diagnostic } => attempt.fail(code, diagnostic)?,
            AttemptTransition::Interrupt { code, diagnostic } => {
                attempt.interrupt(code, diagnostic)?
            }
            AttemptTransition::MarkStale { diagnostic } => attempt.stale(diagnostic)?,
            AttemptTransition::RecordHostFailure { code, diagnostic } => {
                attempt.record_host_failure(code, diagnostic)?
            }
            AttemptTransition::Cancel => attempt.cancel()?,
            AttemptTransition::FinishCancel => attempt.finish_cancel()?,
            AttemptTransition::RecordLog { value } => attempt.record_log(value)?,
            AttemptTransition::RecordProgress { completed, total } => {
                attempt.record_progress(completed, total)?
            }
        }
        let result = attempt.clone();
        self.persist_locked(&database)?;
        Ok(result)
    }

    pub fn retry(&self, attempt_id: &str) -> Result<ResearchAttempt, PythonResearchError> {
        let mut database = self.lock()?;
        let previous = database
            .attempts
            .get(attempt_id)
            .cloned()
            .ok_or_else(|| invalid("research-attempt-not-found"))?;
        database.next_sequence = database.next_sequence.saturating_add(1);
        let next = previous.retry(database.next_sequence)?;
        database
            .attempts
            .insert(next.attempt_id.clone(), next.clone());
        self.persist_locked(&database)?;
        Ok(next)
    }

    fn recover_running(&self) -> Result<(), PythonResearchError> {
        let mut database = self.lock()?;
        let mut changed = false;
        for attempt in database.attempts.values_mut() {
            if attempt.status == AttemptStatus::Running {
                if attempt.cancel_requested {
                    attempt.finish_cancel()?;
                } else {
                    attempt.interrupt(
                        "research-interrupted",
                        "Runner was not terminal at application restart",
                    )?;
                }
                changed = true;
            }
        }
        if changed {
            self.persist_locked(&database)?;
        }
        Ok(())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, AttemptDatabase>, PythonResearchError> {
        self.database
            .lock()
            .map_err(|_| invalid("research-attempt-store-lock-poisoned"))
    }

    fn persist_locked(&self, database: &AttemptDatabase) -> Result<(), PythonResearchError> {
        persist_json(&self.path, database)
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct TrustDatabase {
    decisions: BTreeMap<String, TrustDecision>,
}

#[derive(Clone)]
pub struct TrustStore {
    path: PathBuf,
    database: Arc<Mutex<TrustDatabase>>,
}

impl TrustStore {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self, PythonResearchError> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let database = if path.is_file() {
            serde_json::from_slice(&fs::read(&path)?)
                .map_err(|error| invalid(format!("trust-store-invalid:{error}")))?
        } else {
            TrustDatabase::default()
        };
        Ok(Self {
            path,
            database: Arc::new(Mutex::new(database)),
        })
    }

    pub fn grant(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
    ) -> Result<TrustDecision, PythonResearchError> {
        validate_user_id(user_id)?;
        if project_id.is_empty() || !is_sha256(revision_sha256) {
            return Err(invalid("trust-decision-identity-invalid"));
        }
        let key = format!("{user_id}:{project_id}:{revision_sha256}");
        let mut database = self
            .database
            .lock()
            .map_err(|_| invalid("trust-store-lock-poisoned"))?;
        if let Some(decision) = database.decisions.get(&key) {
            return Ok(decision.clone());
        }
        let decision = TrustDecision {
            decision_id: sha256(key.as_bytes()),
            project_id: project_id.into(),
            revision_sha256: revision_sha256.into(),
            user_id: user_id.into(),
            decided_at_ms: now_ms(),
        };
        database.decisions.insert(key, decision.clone());
        persist_json(&self.path, &*database)?;
        Ok(decision)
    }

    pub fn get(
        &self,
        user_id: &str,
        project_id: &str,
        revision_sha256: &str,
    ) -> Result<Option<TrustDecision>, PythonResearchError> {
        validate_user_id(user_id)?;
        if !is_sha256(revision_sha256) {
            return Err(invalid("trust-decision-identity-invalid"));
        }
        let key = format!("{user_id}:{project_id}:{revision_sha256}");
        Ok(self
            .database
            .lock()
            .map_err(|_| invalid("trust-store-lock-poisoned"))?
            .decisions
            .get(&key)
            .cloned())
    }

    pub fn reset_user(&self, user_id: &str) -> Result<(), PythonResearchError> {
        validate_user_id(user_id)?;
        let mut database = self
            .database
            .lock()
            .map_err(|_| invalid("trust-store-lock-poisoned"))?;
        let prefix = format!("{user_id}:");
        database
            .decisions
            .retain(|key, _| !key.starts_with(&prefix));
        persist_json(&self.path, &*database)
    }
}

pub enum AttemptTransition {
    Begin,
    Complete { result_sha256: String },
    Fail { code: String, diagnostic: String },
    Interrupt { code: String, diagnostic: String },
    MarkStale { diagnostic: String },
    RecordHostFailure { code: String, diagnostic: String },
    Cancel,
    FinishCancel,
    RecordLog { value: String },
    RecordProgress { completed: u64, total: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CancellationState {
    Active,
    CooperativeRequested,
    ForceRequired,
    Terminated,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CancellationController {
    state: CancellationState,
}

impl Default for CancellationController {
    fn default() -> Self {
        Self {
            state: CancellationState::Active,
        }
    }
}

impl CancellationController {
    pub fn state(self) -> CancellationState {
        self.state
    }

    pub fn request(&mut self) {
        if self.state == CancellationState::Active {
            self.state = CancellationState::CooperativeRequested;
        }
    }

    pub fn grace_expired(&mut self) {
        if self.state == CancellationState::CooperativeRequested {
            self.state = CancellationState::ForceRequired;
        }
    }

    pub fn terminated(&mut self) {
        if matches!(
            self.state,
            CancellationState::CooperativeRequested | CancellationState::ForceRequired
        ) {
            self.state = CancellationState::Terminated;
        }
    }

    pub fn may_publish(self) -> bool {
        self.state == CancellationState::Active
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BoundedLog {
    bytes: Vec<u8>,
    limit: usize,
    truncated: bool,
}

impl BoundedLog {
    pub fn new(limit: usize) -> Result<Self, PythonResearchError> {
        if limit == 0 || limit > MAX_LOG_BYTES {
            return Err(invalid("runner-log-limit-invalid"));
        }
        Ok(Self {
            bytes: Vec::new(),
            limit,
            truncated: false,
        })
    }

    pub fn append(&mut self, line: &str) {
        let redacted = redact(line);
        let remaining = self.limit.saturating_sub(self.bytes.len());
        if remaining == 0 {
            self.truncated = true;
            return;
        }
        let bytes = redacted.as_bytes();
        let take = bytes.len().min(remaining);
        self.bytes.extend_from_slice(&bytes[..take]);
        if take < bytes.len() {
            self.truncated = true;
        }
    }

    pub fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub fn truncated(&self) -> bool {
        self.truncated
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrivateChildEnvironment {
    variables: BTreeMap<String, String>,
}

impl PrivateChildEnvironment {
    pub fn from_allowlist(input: BTreeMap<String, String>) -> Result<Self, PythonResearchError> {
        let allowed = [
            "PYTHONHASHSEED",
            "OMP_NUM_THREADS",
            "OPENBLAS_NUM_THREADS",
            "MKL_NUM_THREADS",
            "NUMEXPR_NUM_THREADS",
        ]
        .into_iter()
        .collect::<BTreeSet<_>>();
        if input.keys().any(|key| !allowed.contains(key.as_str())) {
            return Err(invalid("runner-environment-variable-not-allowlisted"));
        }
        Ok(Self { variables: input })
    }

    pub fn variables(&self) -> &BTreeMap<String, String> {
        &self.variables
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StagedCell {
    pub value: Option<f64>,
    pub unavailable_reason: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StagedRow {
    pub row_key: String,
    pub cells: BTreeMap<String, StagedCell>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StagedArtifact {
    pub attempt_id: String,
    pub relative_path: String,
    pub media_type: String,
    pub byte_size: u64,
    pub sha256: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub columns: Option<Vec<ArrowColumn>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArrowColumn {
    pub name: String,
    pub data_type: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ArrowTableDescriptor {
    pub artifact: StagedArtifact,
    pub columns: Vec<ArrowColumn>,
    pub row_count: u64,
}

pub fn validate_arrow_ipc(
    root: &Path,
    descriptor: &ArrowTableDescriptor,
    expected_attempt_id: &str,
    max_bytes: usize,
) -> Result<(), PythonResearchError> {
    if descriptor.artifact.media_type != "application/vnd.apache.arrow.stream"
        || descriptor.columns.is_empty()
        || descriptor.columns[0].name != "rowKey"
    {
        return Err(invalid("runner-arrow-descriptor-invalid"));
    }
    let bytes = read_staged_artifact(root, &descriptor.artifact, expected_attempt_id, max_bytes)?;
    let mut reader = StreamReader::try_new(Cursor::new(bytes), None)
        .map_err(|error| invalid(format!("runner-arrow-stream-invalid:{error}")))?;
    let schema = reader.schema();
    let columns = schema
        .fields()
        .iter()
        .map(|field| ArrowColumn {
            name: field.name().clone(),
            data_type: format!("{:?}", field.data_type()),
        })
        .collect::<Vec<_>>();
    if columns != descriptor.columns {
        return Err(invalid("runner-arrow-schema-invalid"));
    }
    let mut row_count = 0u64;
    let mut previous_key = None;
    for batch in &mut reader {
        let batch =
            batch.map_err(|error| invalid(format!("runner-arrow-batch-invalid:{error}")))?;
        if batch.num_columns() != columns.len() {
            return Err(invalid("runner-arrow-column-count-invalid"));
        }
        let keys = batch
            .column(0)
            .as_any()
            .downcast_ref::<StringArray>()
            .ok_or_else(|| invalid("runner-arrow-row-key-type-invalid"))?;
        for row in 0..batch.num_rows() {
            let key = keys
                .is_valid(row)
                .then(|| keys.value(row).to_owned())
                .ok_or_else(|| invalid("runner-arrow-row-key-missing"))?;
            if previous_key
                .as_deref()
                .is_some_and(|previous| previous >= key.as_str())
            {
                return Err(invalid("runner-arrow-row-order-invalid"));
            }
            previous_key = Some(key);
        }
        for (index, field) in schema.fields().iter().enumerate() {
            match field.data_type() {
                DataType::Float32 => {
                    let values = batch
                        .column(index)
                        .as_any()
                        .downcast_ref::<Float32Array>()
                        .ok_or_else(|| invalid("runner-arrow-float-type-invalid"))?;
                    if (0..values.len())
                        .any(|row| values.is_valid(row) && !values.value(row).is_finite())
                    {
                        return Err(invalid("runner-arrow-non-finite-value"));
                    }
                }
                DataType::Float64 => {
                    let values = batch
                        .column(index)
                        .as_any()
                        .downcast_ref::<Float64Array>()
                        .ok_or_else(|| invalid("runner-arrow-float-type-invalid"))?;
                    if (0..values.len())
                        .any(|row| values.is_valid(row) && !values.value(row).is_finite())
                    {
                        return Err(invalid("runner-arrow-non-finite-value"));
                    }
                }
                _ => {}
            }
        }
        row_count = row_count
            .checked_add(batch.num_rows() as u64)
            .ok_or_else(|| invalid("runner-arrow-row-count-overflow"))?;
        if row_count > MAX_RESULT_ROWS as u64 {
            return Err(invalid("runner-arrow-row-limit-exceeded"));
        }
    }
    if row_count != descriptor.row_count {
        return Err(invalid("runner-arrow-row-count-mismatch"));
    }
    Ok(())
}

pub fn read_staged_artifact(
    root: &Path,
    artifact: &StagedArtifact,
    expected_attempt_id: &str,
    max_bytes: usize,
) -> Result<Vec<u8>, PythonResearchError> {
    if artifact.attempt_id != expected_attempt_id
        || artifact.relative_path.is_empty()
        || !matches!(
            artifact.media_type.as_str(),
            "application/json" | "application/vnd.apache.arrow.stream" | "application/octet-stream"
        )
        || artifact.byte_size > max_bytes as u64
        || !is_sha256(&artifact.sha256)
    {
        return Err(invalid("runner-staged-artifact-identity-invalid"));
    }
    let relative = Path::new(&artifact.relative_path);
    let mut path = root.to_path_buf();
    for component in relative.components() {
        let Component::Normal(component) = component else {
            return Err(invalid("runner-staged-artifact-path-invalid"));
        };
        path.push(component);
        let metadata =
            fs::symlink_metadata(&path).map_err(|_| invalid("runner-staged-artifact-not-found"))?;
        if metadata.file_type().is_symlink() {
            return Err(invalid("runner-staged-artifact-symlink-not-allowed"));
        }
    }
    let file = fs::File::open(&path).map_err(|_| invalid("runner-staged-artifact-not-found"))?;
    let metadata = file
        .metadata()
        .map_err(|_| invalid("runner-staged-artifact-not-found"))?;
    if !metadata.is_file() {
        return Err(invalid("runner-staged-artifact-not-file"));
    }
    #[cfg(unix)]
    if std::os::unix::fs::MetadataExt::nlink(&metadata) > 1 {
        return Err(invalid("runner-staged-artifact-hard-link-not-allowed"));
    }
    let mut bytes = Vec::new();
    file.take((max_bytes as u64).saturating_add(1))
        .read_to_end(&mut bytes)?;
    if bytes.len() as u64 != artifact.byte_size || sha256(&bytes) != artifact.sha256 {
        return Err(invalid("runner-staged-artifact-hash-invalid"));
    }
    Ok(bytes)
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StagedResult {
    pub attempt_id: String,
    pub output_names: Vec<String>,
    pub rows: Vec<StagedRow>,
    pub payload_sha256: String,
}

impl StagedResult {
    pub fn validate(
        &self,
        expected_attempt_id: &str,
        expected_outputs: &[String],
    ) -> Result<(), PythonResearchError> {
        if self.attempt_id != expected_attempt_id
            || self.output_names != expected_outputs
            || self.rows.len() > MAX_RESULT_ROWS
        {
            return Err(invalid("runner-result-identity-or-size-invalid"));
        }
        let mut previous = None;
        for row in &self.rows {
            if row.row_key.is_empty() || previous.is_some_and(|key| key >= row.row_key.as_str()) {
                return Err(invalid("runner-result-row-order-invalid"));
            }
            previous = Some(row.row_key.as_str());
            if row.cells.len() != expected_outputs.len()
                || row
                    .cells
                    .keys()
                    .any(|name| !expected_outputs.contains(name))
            {
                return Err(invalid("runner-result-schema-invalid"));
            }
            for cell in row.cells.values() {
                match (cell.value, cell.unavailable_reason.as_deref()) {
                    (Some(value), None) if value.is_finite() => {}
                    (None, Some(reason)) if !reason.is_empty() => {}
                    _ => return Err(invalid("runner-result-value-invalid")),
                }
            }
        }
        let mut copy = self.clone();
        copy.payload_sha256.clear();
        let bytes = serde_json::to_vec(&copy).map_err(|error| invalid(error.to_string()))?;
        if self.payload_sha256 != sha256(&bytes) {
            return Err(invalid("runner-result-hash-invalid"));
        }
        Ok(())
    }
}

fn bounded_diagnostic(value: &str) -> String {
    redact(value).chars().take(4096).collect()
}

fn redact(value: &str) -> String {
    value
        .split_whitespace()
        .map(|part| {
            let upper = part.to_ascii_uppercase();
            if ["TOKEN", "SECRET", "PASSWORD", "PRIVATE_KEY"]
                .iter()
                .any(|key| upper.contains(key))
            {
                "[redacted]"
            } else if part.starts_with('/') || part.contains('\\') {
                "[path]"
            } else {
                part
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn persist_json<T: Serialize>(path: &PathBuf, value: &T) -> Result<(), PythonResearchError> {
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(value).map_err(|error| invalid(error.to_string()))?,
    )?;
    fs::rename(temporary, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn hash(seed: &str) -> String {
        sha256(seed.as_bytes())
    }

    fn policy() -> HostResourcePolicy {
        HostResourcePolicy::m12_default()
    }

    #[test]
    fn handshake_and_framing_fail_closed() {
        let token = "x".repeat(32);
        let message = ControlMessage::Hello {
            handshake: Handshake {
                protocol: RUNNER_PROTOCOL_VERSION.into(),
                sdk_artifact_sha256: hash("sdk"),
                revision_sha256: hash("revision"),
                environment_sha256: hash("environment"),
                attempt_id: "attempt".into(),
                loopback: true,
                one_time_token: token.clone(),
            },
        };
        let frame = encode_control(&message).unwrap();
        assert!(matches!(
            decode_control(&frame).unwrap(),
            ControlMessage::Hello { .. }
        ));
        assert!(decode_control(&[0, 0, 0, 1]).is_err());
        if let ControlMessage::Hello { handshake } = message {
            handshake
                .validate(
                    &hash("sdk"),
                    &hash("revision"),
                    &hash("environment"),
                    "attempt",
                    &token,
                )
                .unwrap();
            assert!(
                handshake
                    .validate(
                        &hash("sdk"),
                        &hash("other"),
                        &hash("environment"),
                        "attempt",
                        &token
                    )
                    .is_err()
            );
        }
    }

    #[test]
    fn staged_artifacts_are_attempt_bound_and_hash_checked() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("result.json");
        let bytes = br#"{"ok":true}"#;
        fs::write(&path, bytes).unwrap();
        let artifact = StagedArtifact {
            attempt_id: "attempt".into(),
            relative_path: "result.json".into(),
            media_type: "application/json".into(),
            byte_size: bytes.len() as u64,
            sha256: sha256(bytes),
            columns: None,
            row_count: None,
        };
        assert_eq!(
            read_staged_artifact(directory.path(), &artifact, "attempt", 1024).unwrap(),
            bytes
        );
        let mut wrong_attempt = artifact.clone();
        wrong_attempt.attempt_id = "other".into();
        assert!(read_staged_artifact(directory.path(), &wrong_attempt, "attempt", 1024).is_err());
        let mut wrong_hash = artifact.clone();
        wrong_hash.sha256 = hash("wrong");
        assert!(read_staged_artifact(directory.path(), &wrong_hash, "attempt", 1024).is_err());
        let mut unsafe_path = artifact.clone();
        unsafe_path.relative_path = "../result.json".into();
        assert!(read_staged_artifact(directory.path(), &unsafe_path, "attempt", 1024).is_err());
        assert!(read_staged_artifact(directory.path(), &artifact, "attempt", 1).is_err());
    }

    #[test]
    fn arrow_streams_preserve_row_identity_schema_and_finite_values() {
        let directory = tempdir().unwrap();
        let schema = std::sync::Arc::new(arrow_schema::Schema::new(vec![
            arrow_schema::Field::new("rowKey", DataType::Utf8, false),
            arrow_schema::Field::new("score", DataType::Float64, true),
        ]));
        let write_stream = |values: Vec<Option<f64>>| {
            let batch = arrow_array::RecordBatch::try_new(
                schema.clone(),
                vec![
                    std::sync::Arc::new(StringArray::from(vec!["a", "b"])),
                    std::sync::Arc::new(Float64Array::from(values)),
                ],
            )
            .unwrap();
            let mut bytes = Vec::new();
            {
                let mut writer =
                    arrow_ipc::writer::StreamWriter::try_new(&mut bytes, &schema).unwrap();
                writer.write(&batch).unwrap();
                writer.finish().unwrap();
            }
            bytes
        };
        let bytes = write_stream(vec![Some(1.0), None]);
        let path = directory.path().join("table.arrow");
        fs::write(&path, &bytes).unwrap();
        let descriptor = ArrowTableDescriptor {
            artifact: StagedArtifact {
                attempt_id: "attempt".into(),
                relative_path: "table.arrow".into(),
                media_type: "application/vnd.apache.arrow.stream".into(),
                byte_size: bytes.len() as u64,
                sha256: sha256(&bytes),
                columns: None,
                row_count: None,
            },
            columns: vec![
                ArrowColumn {
                    name: "rowKey".into(),
                    data_type: "Utf8".into(),
                },
                ArrowColumn {
                    name: "score".into(),
                    data_type: "Float64".into(),
                },
            ],
            row_count: 2,
        };
        validate_arrow_ipc(directory.path(), &descriptor, "attempt", 4096).unwrap();

        let bad_bytes = write_stream(vec![Some(f64::NAN), None]);
        fs::write(&path, &bad_bytes).unwrap();
        let mut bad_descriptor = descriptor;
        bad_descriptor.artifact.byte_size = bad_bytes.len() as u64;
        bad_descriptor.artifact.sha256 = sha256(&bad_bytes);
        assert!(validate_arrow_ipc(directory.path(), &bad_descriptor, "attempt", 4096).is_err());
    }

    fn test_python_executable(message: &str) -> PathBuf {
        let names: &[&str] = if cfg!(windows) {
            &["python.exe", "python3.exe", "python"]
        } else {
            &["python3", "python"]
        };
        std::env::var_os("ADAQ_TEST_PYTHON")
            .map(PathBuf::from)
            .or_else(|| {
                std::env::var_os("PATH").and_then(|path| {
                    std::env::split_paths(&path)
                        .flat_map(|directory| names.iter().map(move |name| directory.join(name)))
                        .find(|candidate| candidate.is_file())
                })
            })
            .expect(message)
    }

    #[test]
    fn loopback_runner_executes_repository_conformance_project() {
        let token = "x".repeat(64);
        let python_executable =
            test_python_executable("Python is required for the runner conformance test");
        let staging = tempdir().unwrap();
        let spec = RunnerLaunchSpec {
            python_executable,
            runner_script: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../python/adaq-python-research-runner/src/adaq_runner/__main__.py"),
            runner_wheel: None,
            project_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../python/runner_tests/conformance_project"),
            entry_point: "project:create_project".into(),
            sdk_wheel: None,
            adapter_wheel: None,
            handshake: Handshake {
                protocol: RUNNER_PROTOCOL_VERSION.into(),
                sdk_artifact_sha256: hash("sdk"),
                revision_sha256: hash("revision"),
                environment_sha256: hash("environment"),
                attempt_id: "conformance-attempt".into(),
                loopback: true,
                one_time_token: token,
            },
            environment: PrivateChildEnvironment::from_allowlist(BTreeMap::new()).unwrap(),
            staging_root: staging.path().to_path_buf(),
            staged_result_path: staging.path().join("conformance-result.json"),
            staged_result_relative_path: "conformance-result.json".into(),
            execution: AttemptExecution {
                sdk_artifact_sha256: hash("sdk"),
                runtime_artifact_sha256: hash("runtime"),
                entry_point: "project:create_project".into(),
                ..AttemptExecution::default()
            },
            seed: 0,
            max_wall_ms: 10_000,
            max_memory_bytes: policy().max_memory_bytes,
            max_processes: policy().max_processes,
            max_control_bytes: MAX_CONTROL_MESSAGE_BYTES,
            max_arrow_bytes: policy().max_arrow_bytes as usize,
            max_staged_bytes: policy().max_staged_bytes as usize,
            max_artifact_bytes: policy().max_artifact_bytes as usize,
            max_log_bytes: 4096,
        };
        let execution = run_process(&spec, || false).unwrap();
        let result = execution.conformance.unwrap();
        assert_eq!(result.attempt_id, "conformance-attempt");
        assert_eq!(result.project_id, "runner-conformance@1");
        assert_eq!(result.project_kind, "conformance");
        assert_eq!(result.entry_point, "project:create_project");
    }

    #[test]
    fn loopback_runner_returns_the_factor_definition_payload() {
        let token = "y".repeat(64);
        let python_executable =
            test_python_executable("Python is required for the runner descriptor test");
        let staging = tempdir().unwrap();
        let spec = RunnerLaunchSpec {
            python_executable,
            runner_script: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../python/adaq-python-research-runner/src/adaq_runner/__main__.py"),
            runner_wheel: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("resources/wheels/adaq_python_research_runner-1.0.0-py3-none-any.whl"),
            ),
            project_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../examples/python/py-factor-cross-sectional-momentum"),
            entry_point: "project:create_project".into(),
            sdk_wheel: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../python/adaq-research-sdk/src"),
            ),
            adapter_wheel: None,
            handshake: Handshake {
                protocol: RUNNER_PROTOCOL_VERSION.into(),
                sdk_artifact_sha256: hash("sdk"),
                revision_sha256: hash("revision"),
                environment_sha256: hash("environment"),
                attempt_id: "factor-descriptor-attempt".into(),
                loopback: true,
                one_time_token: token,
            },
            environment: PrivateChildEnvironment::from_allowlist(BTreeMap::new()).unwrap(),
            staging_root: staging.path().to_path_buf(),
            staged_result_path: staging.path().join("conformance-result.json"),
            staged_result_relative_path: "conformance-result.json".into(),
            execution: AttemptExecution {
                sdk_artifact_sha256: hash("sdk"),
                runtime_artifact_sha256: hash("runtime"),
                entry_point: "project:create_project".into(),
                ..AttemptExecution::default()
            },
            seed: 0,
            max_wall_ms: 10_000,
            max_memory_bytes: policy().max_memory_bytes,
            max_processes: policy().max_processes,
            max_control_bytes: MAX_CONTROL_MESSAGE_BYTES,
            max_arrow_bytes: policy().max_arrow_bytes as usize,
            max_staged_bytes: policy().max_staged_bytes as usize,
            max_artifact_bytes: policy().max_artifact_bytes as usize,
            max_log_bytes: 4096,
        };
        let result = run_process(&spec, || false).unwrap().conformance.unwrap();
        assert_eq!(result.project_kind, "factor");
        let payload = result.payload.unwrap();
        assert_eq!(payload["definition"]["scope"], "cross-sectional");
        assert_eq!(payload["definition"]["outputs"][0], "momentum-score");
    }

    #[test]
    fn loopback_runner_fits_only_the_host_fed_model_partitions() {
        let token = "m".repeat(64);
        let python_executable =
            test_python_executable("Python is required for the model adapter test");
        let staging = tempdir().unwrap();
        let rows = |start: i64, end: i64, labels: bool| {
            (start..=end)
                .map(|datetime| crate::model::HostPartitionRow {
                    datetime,
                    instrument: "AAA".into(),
                    features: vec![datetime as f64],
                    label: labels.then_some(datetime as f64),
                })
                .collect::<Vec<_>>()
        };
        let mut train = rows(1, 3, true);
        train[1].label = None;
        let mut valid = rows(4, 5, true);
        valid[1].label = None;
        let transformation =
            crate::model::FittedTransformation::fit(&train, &["momentum-score".into()]).unwrap();
        let input = crate::model::ModelRunnerInput {
            train,
            valid,
            test: rows(6, 6, false),
            train_labels: vec![
                crate::model::TargetLabel::Value(1.0),
                crate::model::TargetLabel::Unavailable {
                    reason: "target-window-boundary".into(),
                },
                crate::model::TargetLabel::Value(3.0),
            ],
            valid_labels: vec![
                crate::model::TargetLabel::Value(4.0),
                crate::model::TargetLabel::Unavailable {
                    reason: "target-window-boundary".into(),
                },
            ],
            transformation,
            fitted_model: None,
            target_window_end: None,
        };
        input.validate().unwrap();
        let spec = RunnerLaunchSpec {
            python_executable,
            runner_script: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../python/adaq-python-research-runner/src/adaq_runner/__main__.py"),
            runner_wheel: None,
            project_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../examples/python/py-model-qlib-ridge-return"),
            entry_point: "project:create_project".into(),
            sdk_wheel: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../python/adaq-research-sdk/src"),
            ),
            adapter_wheel: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../python/adaq-qlib-ridge-adapter/src"),
            ),
            handshake: Handshake {
                protocol: RUNNER_PROTOCOL_VERSION.into(),
                sdk_artifact_sha256: hash("sdk"),
                revision_sha256: hash("revision"),
                environment_sha256: hash("environment"),
                attempt_id: "model-adapter-attempt".into(),
                loopback: true,
                one_time_token: token,
            },
            environment: PrivateChildEnvironment::from_allowlist(BTreeMap::new()).unwrap(),
            staging_root: staging.path().to_path_buf(),
            staged_result_path: staging.path().join("conformance-result.json"),
            staged_result_relative_path: "conformance-result.json".into(),
            execution: AttemptExecution {
                sdk_artifact_sha256: hash("sdk"),
                runtime_artifact_sha256: hash("runtime"),
                entry_point: "project:create_project".into(),
                parameters: BTreeMap::from([("alpha".into(), "1".into())]),
                seed: 7,
                output_names: vec!["forecast".into()],
                input: Some(serde_json::to_value(input).unwrap()),
                ..AttemptExecution::default()
            },
            seed: 7,
            max_wall_ms: 10_000,
            max_memory_bytes: policy().max_memory_bytes,
            max_processes: policy().max_processes,
            max_control_bytes: MAX_CONTROL_MESSAGE_BYTES,
            max_arrow_bytes: policy().max_arrow_bytes as usize,
            max_staged_bytes: policy().max_staged_bytes as usize,
            max_artifact_bytes: policy().max_artifact_bytes as usize,
            max_log_bytes: 4096,
        };
        let result = run_process(&spec, || false).unwrap().conformance.unwrap();
        let payload = result.payload.unwrap();
        assert_eq!(payload["fit"]["schema"], "adaq:linear-model:candidate@1");
        assert_eq!(
            payload["fit"]["payload"]["adapter_id"],
            "qlib-linear-ridge@1"
        );
        assert!(payload["fit"]["payload"].get("coefficients").is_some());
    }

    #[test]
    fn loopback_runner_preserves_imperative_factor_batches_and_missing_input() {
        let token = "z".repeat(64);
        let python_executable =
            test_python_executable("Python is required for the imperative Factor test");
        let staging = tempdir().unwrap();
        let input = crate::factor::PythonFactorInput {
            universe: vec!["AAA".into(), "BBB".into()],
            segments: vec![
                crate::factor::PythonFactorSegment {
                    segment_id: "continuous-1".into(),
                    batches: vec![
                        crate::factor::PythonFactorBatch {
                            rows: vec![
                                crate::factor::MomentumInputRow {
                                    instrument_id: "AAA".into(),
                                    observation_time_ms: 1,
                                    close: Some(1.0),
                                },
                                crate::factor::MomentumInputRow {
                                    instrument_id: "BBB".into(),
                                    observation_time_ms: 1,
                                    close: Some(2.0),
                                },
                            ],
                        },
                        crate::factor::PythonFactorBatch {
                            rows: vec![
                                crate::factor::MomentumInputRow {
                                    instrument_id: "AAA".into(),
                                    observation_time_ms: 2,
                                    close: None,
                                },
                                crate::factor::MomentumInputRow {
                                    instrument_id: "BBB".into(),
                                    observation_time_ms: 2,
                                    close: Some(3.0),
                                },
                            ],
                        },
                    ],
                },
                crate::factor::PythonFactorSegment {
                    segment_id: "after-gap".into(),
                    batches: vec![crate::factor::PythonFactorBatch {
                        rows: vec![
                            crate::factor::MomentumInputRow {
                                instrument_id: "AAA".into(),
                                observation_time_ms: 3,
                                close: Some(4.0),
                            },
                            crate::factor::MomentumInputRow {
                                instrument_id: "BBB".into(),
                                observation_time_ms: 3,
                                close: Some(5.0),
                            },
                        ],
                    }],
                },
            ],
        };
        let execution = AttemptExecution {
            sdk_artifact_sha256: hash("sdk"),
            runtime_artifact_sha256: hash("runtime"),
            entry_point: "project:create_project".into(),
            parameters: BTreeMap::from([("lookback".into(), "20".into())]),
            seed: 7,
            output_names: vec!["momentum-score".into()],
            input: Some(serde_json::to_value(&input).unwrap()),
            ..AttemptExecution::default()
        };
        let spec = RunnerLaunchSpec {
            python_executable,
            runner_script: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../python/adaq-python-research-runner/src/adaq_runner/__main__.py"),
            runner_wheel: None,
            project_root: PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../../python/runner_tests/imperative_factor_project"),
            entry_point: "project:create_project".into(),
            sdk_wheel: Some(
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../../python/adaq-research-sdk/src"),
            ),
            adapter_wheel: None,
            handshake: Handshake {
                protocol: RUNNER_PROTOCOL_VERSION.into(),
                sdk_artifact_sha256: hash("sdk"),
                revision_sha256: hash("revision"),
                environment_sha256: hash("environment"),
                attempt_id: "imperative-factor-attempt".into(),
                loopback: true,
                one_time_token: token,
            },
            environment: PrivateChildEnvironment::from_allowlist(BTreeMap::new()).unwrap(),
            staging_root: staging.path().to_path_buf(),
            staged_result_path: staging.path().join("conformance-result.json"),
            staged_result_relative_path: "conformance-result.json".into(),
            execution,
            seed: 7,
            max_wall_ms: 10_000,
            max_memory_bytes: policy().max_memory_bytes,
            max_processes: policy().max_processes,
            max_control_bytes: MAX_CONTROL_MESSAGE_BYTES,
            max_arrow_bytes: policy().max_arrow_bytes as usize,
            max_staged_bytes: policy().max_staged_bytes as usize,
            max_artifact_bytes: policy().max_artifact_bytes as usize,
            max_log_bytes: 4096,
        };
        let payload = run_process(&spec, || false)
            .unwrap()
            .conformance
            .unwrap()
            .payload
            .unwrap();
        let rows = crate::factor::validate_imperative_factor_payload(&payload, &input).unwrap();
        assert_eq!(rows.len(), 6);
        assert_eq!(
            rows[2].unavailable_reason,
            Some(crate::factor::FactorUnavailableReason::MissingInput)
        );
    }

    #[test]
    fn persistent_attempts_recover_retry_and_coalesce() {
        let directory = tempdir().unwrap();
        let store = AttemptStore::open(directory.path().join("attempts.json")).unwrap();
        let first = store
            .enqueue(
                "alice",
                "runner-conformance@1",
                hash("revision"),
                hash("environment"),
                policy(),
            )
            .unwrap();
        let same = store
            .enqueue(
                "alice",
                "runner-conformance@1",
                hash("revision"),
                hash("environment"),
                policy(),
            )
            .unwrap();
        assert_eq!(first.attempt_id, same.attempt_id);
        assert!(
            store
                .active_environment_ids()
                .unwrap()
                .contains(&hash("environment"))
        );
        store
            .transition(&first.attempt_id, AttemptTransition::Begin)
            .unwrap();
        drop(store);
        let store = AttemptStore::open(directory.path().join("attempts.json")).unwrap();
        assert_eq!(
            store.list("alice").unwrap()[0].status,
            AttemptStatus::Interrupted
        );
        let retry = store.retry(&first.attempt_id).unwrap();
        assert_eq!(
            retry.source_attempt_id.as_deref(),
            Some(first.attempt_id.as_str())
        );
        assert!(
            store
                .active_environment_ids()
                .unwrap()
                .contains(&hash("environment"))
        );
    }

    #[test]
    fn stale_and_host_failed_attempts_remain_retryable_with_bounded_diagnostics() {
        let directory = tempdir().unwrap();
        let store = AttemptStore::open(directory.path().join("attempts.json")).unwrap();
        let stale = store
            .enqueue(
                "alice",
                "runner-conformance@1",
                hash("stale-revision"),
                hash("stale-environment"),
                policy(),
            )
            .unwrap();
        store
            .transition(
                &stale.attempt_id,
                AttemptTransition::MarkStale {
                    diagnostic: "stale response".into(),
                },
            )
            .unwrap();
        assert_eq!(
            store.get(&stale.attempt_id).unwrap().status,
            AttemptStatus::Stale
        );

        let host_failed = store
            .enqueue(
                "alice",
                "runner-conformance@1",
                hash("host-failure-revision"),
                hash("host-failure-environment"),
                policy(),
            )
            .unwrap();
        store
            .transition(&host_failed.attempt_id, AttemptTransition::Begin)
            .unwrap();
        store
            .transition(
                &host_failed.attempt_id,
                AttemptTransition::Complete {
                    result_sha256: hash("result"),
                },
            )
            .unwrap();
        store
            .transition(
                &host_failed.attempt_id,
                AttemptTransition::RecordHostFailure {
                    code: "model-host-save-failed".into(),
                    diagnostic: "x".repeat(5000),
                },
            )
            .unwrap();
        let recorded = store.get(&host_failed.attempt_id).unwrap();
        assert_eq!(recorded.status, AttemptStatus::Completed);
        assert_eq!(
            recorded.failure_code.as_deref(),
            Some("model-host-save-failed")
        );
        assert_eq!(recorded.diagnostic.as_ref().unwrap().len(), 4096);

        assert_eq!(
            store.retry(&stale.attempt_id).unwrap().source_attempt_id,
            Some(stale.attempt_id)
        );
        assert_eq!(
            store
                .retry(&host_failed.attempt_id)
                .unwrap()
                .source_attempt_id,
            Some(host_failed.attempt_id)
        );
    }

    #[test]
    fn cancelling_a_user_marks_pending_and_running_attempts_before_reset() {
        let directory = tempdir().unwrap();
        let store = AttemptStore::open(directory.path().join("attempts.json")).unwrap();
        let pending = store
            .enqueue(
                "alice",
                "runner-conformance@1",
                hash("pending-revision"),
                hash("pending-environment"),
                policy(),
            )
            .unwrap();
        let running = store
            .enqueue(
                "alice",
                "runner-factor@1",
                hash("running-revision"),
                hash("running-environment"),
                policy(),
            )
            .unwrap();
        store
            .transition(&running.attempt_id, AttemptTransition::Begin)
            .unwrap();
        store.cancel_user("alice").unwrap();
        assert_eq!(
            store.get(&pending.attempt_id).unwrap().status,
            AttemptStatus::Cancelled
        );
        assert!(store.get(&running.attempt_id).unwrap().cancel_requested);
        assert!(store.has_active_for_user("alice").unwrap());
        store
            .transition(&running.attempt_id, AttemptTransition::FinishCancel)
            .unwrap();
        assert!(!store.has_active_for_user("alice").unwrap());
    }

    #[test]
    fn attempt_progress_is_bounded_and_terminal() {
        let directory = tempdir().unwrap();
        let store = AttemptStore::open(directory.path().join("attempts.json")).unwrap();
        let attempt = store
            .enqueue(
                "alice",
                "runner-conformance@1",
                hash("revision"),
                hash("environment"),
                policy(),
            )
            .unwrap();
        store
            .transition(&attempt.attempt_id, AttemptTransition::Begin)
            .unwrap();
        store
            .transition(
                &attempt.attempt_id,
                AttemptTransition::RecordProgress {
                    completed: 2,
                    total: 3,
                },
            )
            .unwrap();
        let current = store.get(&attempt.attempt_id).unwrap();
        assert_eq!((current.progress_completed, current.progress_total), (2, 3));
        assert!(
            store
                .transition(
                    &attempt.attempt_id,
                    AttemptTransition::RecordProgress {
                        completed: 4,
                        total: 3,
                    },
                )
                .is_err()
        );
        store
            .transition(
                &attempt.attempt_id,
                AttemptTransition::Complete {
                    result_sha256: hash("result"),
                },
            )
            .unwrap();
        let completed = store.get(&attempt.attempt_id).unwrap();
        assert_eq!(
            (completed.progress_completed, completed.progress_total),
            (3, 3)
        );
    }

    #[test]
    fn cancellation_and_resource_policy_are_host_owned() {
        let mut cancellation = CancellationController::default();
        cancellation.request();
        assert!(!cancellation.may_publish());
        cancellation.grace_expired();
        cancellation.terminated();
        assert_eq!(cancellation.state(), CancellationState::Terminated);
        let mut request = policy();
        request.max_threads = policy().max_threads + 1;
        assert!(policy().lowered_by(&request).is_err());
        let mut project_request = crate::ResourceRequest {
            max_wall_ms: 1000,
            max_memory_bytes: 1024 * 1024,
            max_threads: 2,
            max_input_rows: 100,
            max_output_rows: 50,
        };
        let lowered = policy().lowered_by_request(&project_request).unwrap();
        assert_eq!(lowered.max_output_rows, 50);
        project_request.max_threads = policy().max_threads + 1;
        assert!(policy().lowered_by_request(&project_request).is_err());
    }

    #[test]
    fn logs_and_private_environment_are_bounded() {
        let mut log = BoundedLog::new(32).unwrap();
        log.append("token=secret123 this message is deliberately much longer than the limit");
        assert!(log.truncated());
        assert!(!String::from_utf8_lossy(log.as_bytes()).contains("secret123"));
        let mut variables = BTreeMap::new();
        variables.insert("PATH".into(), "/bad".into());
        assert!(PrivateChildEnvironment::from_allowlist(variables).is_err());
    }

    #[test]
    fn staged_results_require_sorted_finite_identity_preserving_rows() {
        let mut result = StagedResult {
            attempt_id: "attempt".into(),
            output_names: vec!["score".into()],
            rows: vec![StagedRow {
                row_key: "2020-01-01|AAA".into(),
                cells: BTreeMap::from([(
                    "score".into(),
                    StagedCell {
                        value: Some(1.0),
                        unavailable_reason: None,
                    },
                )]),
            }],
            payload_sha256: String::new(),
        };
        let bytes = serde_json::to_vec(&result).unwrap();
        result.payload_sha256 = sha256(&bytes);
        result.validate("attempt", &["score".into()]).unwrap();
        result.rows[0].cells.get_mut("score").unwrap().value = Some(f64::NAN);
        assert!(result.validate("attempt", &["score".into()]).is_err());
    }
}
