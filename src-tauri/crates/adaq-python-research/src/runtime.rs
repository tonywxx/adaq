//! Managed Runtime, Wheelhouse, Lock, and Environment contracts.
//!
//! This module treats downloaded bytes as opaque signed archives. It never
//! discovers an interpreter, invokes Python, runs a wheel build backend, or
//! contacts an index.

use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use flate2::read::GzDecoder;
use hmac::{Hmac, KeyInit, Mac};
use serde::{Deserialize, Serialize};
use tar::Archive;
use zip::ZipArchive;

use crate::{PythonResearchError, invalid, is_sha256, sha256};

pub const RUNTIME_MANIFEST_SCHEMA: &str = "adaq-runtime-manifest@1";
pub const WHEELHOUSE_MANIFEST_SCHEMA: &str = "adaq-wheelhouse-manifest@1";
pub const ENVIRONMENT_LOCK_SCHEMA: &str = "adaq-environment-lock@1";
pub const PYTHON_RUNTIME_PROFILE: &str = "adaq-python@1";
pub const TRUSTED_RUNTIME_SIGNING_KEY: &[u8] = b"adaq-managed-runtime-signing-key-v1";
pub const REQUIRED_WHEEL_PACKAGES: [&str; 5] = [
    "adaq-research-sdk",
    "adaq-python-research-runner",
    "adaq-qlib-ridge-adapter",
    "pyarrow",
    "numpy",
];
const MAX_RUNTIME_ARCHIVE_EXPANDED_BYTES: u64 = 512 * 1024 * 1024;
const MAX_RUNTIME_ARCHIVE_ENTRY_BYTES: u64 = 256 * 1024 * 1024;
const PYTHON_STANDALONE_RELEASE_TAG: &str = "20260807";

type HmacSha256 = Hmac<sha2::Sha256>;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RuntimePlatform {
    MacosAarch64,
    WindowsX86_64,
    LinuxX86_64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeCatalogEntry {
    pub manifest: RuntimeArtifactManifest,
    pub download_url: String,
}

pub fn runtime_catalog() -> Vec<RuntimeCatalogEntry> {
    [
        (
            RuntimePlatform::MacosAarch64,
            "cpython-3.12.13+20260807-aarch64-apple-darwin-install_only.tar.gz",
            25_168_985,
            66_325_948,
            "4201588fc5051c2ba988abbe1f033d318965ee378fadf7fb7ef79882ba7be84b",
        ),
        (
            RuntimePlatform::WindowsX86_64,
            "cpython-3.12.13+20260807-x86_64-pc-windows-msvc-install_only.tar.gz",
            46_113_914,
            149_242_131,
            "6cf2be701aa7e9470454c9c86285c1bcc1832518d63e39c3e34e9d8ea1cbb99f",
        ),
        (
            RuntimePlatform::LinuxX86_64,
            "cpython-3.12.13+20260807-x86_64-unknown-linux-gnu-install_only.tar.gz",
            109_285_525,
            350_100_838,
            "5bd6f36fd7ef02b909234c94dca9994ef0da06ace3bc3cece4fe27870e9cdbbe",
        ),
    ]
    .into_iter()
    .map(
        |(platform, file_name, download_bytes, installed_bytes, artifact_sha256)| {
            let mut manifest = RuntimeArtifactManifest {
                    schema: RUNTIME_MANIFEST_SCHEMA.into(),
                    profile: PYTHON_RUNTIME_PROFILE.into(),
                    version: "3.12.13".into(),
                    platform,
                    source: format!(
                        "https://github.com/astral-sh/python-build-standalone/releases/tag/{PYTHON_STANDALONE_RELEASE_TAG}"
                    ),
                    license: "PSF-2.0".into(),
                    file_name: file_name.into(),
                    download_bytes,
                    installed_bytes,
                    artifact_sha256: artifact_sha256.into(),
                    signature: String::new(),
                };
            manifest.signature = manifest
                .expected_signature()
                .expect("managed runtime catalog signing key is valid");
            RuntimeCatalogEntry {
                manifest,
                download_url: format!(
                    "https://github.com/astral-sh/python-build-standalone/releases/download/{PYTHON_STANDALONE_RELEASE_TAG}/{}",
                    file_name.replace('+', "%2B")
                ),
            }
        },
    )
    .collect()
}

pub fn runtime_catalog_entry(
    platform: RuntimePlatform,
) -> Result<RuntimeCatalogEntry, PythonResearchError> {
    runtime_catalog()
        .into_iter()
        .find(|entry| entry.manifest.platform == platform)
        .ok_or_else(|| invalid("python-runtime-catalog-platform-missing"))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WheelhouseCatalogEntry {
    pub manifest: WheelhouseManifest,
    pub download_urls: BTreeMap<String, String>,
}

pub fn wheelhouse_catalog(
    platform: RuntimePlatform,
) -> Result<WheelhouseCatalogEntry, PythonResearchError> {
    let sdk = WheelIdentity {
        file_name: "adaq_research_sdk-1.0.0-py3-none-any.whl".into(),
        package: "adaq-research-sdk".into(),
        version: "1.0.0".into(),
        sha256: "54cb0dd8f1b2f911a30099f1c7ffdc3798cd3d18e7a331b6708b437f6fa28ed7".into(),
        size: 7_627,
        platform_tags: vec!["any".into()],
    };
    let runner = WheelIdentity {
        file_name: "adaq_python_research_runner-1.0.0-py3-none-any.whl".into(),
        package: "adaq-python-research-runner".into(),
        version: "1.0.0".into(),
        sha256: "e84481af16dcbf20c6ee80b75d507c41c31b57166cb4aeabdbc0f9d9195c7f18".into(),
        size: 6_813,
        platform_tags: vec!["any".into()],
    };
    let qlib_adapter = WheelIdentity {
        file_name: "adaq_qlib_ridge_adapter-1.0.0-py3-none-any.whl".into(),
        package: "adaq-qlib-ridge-adapter".into(),
        version: "1.0.0".into(),
        sha256: "fc94acb8da4eab60b5a677354d9d2bcd1a0fcbc6fde249a2f080869e50ed50e4".into(),
        size: 3_322,
        platform_tags: vec!["any".into()],
    };
    let (numpy, arrow) = match platform {
        RuntimePlatform::MacosAarch64 => (
            (
                "numpy-2.1.3-cp312-cp312-macosx_14_0_arm64.whl",
                "a6b46587b14b888e95e4a24d7b13ae91fa22386c199ee7b418f449032b2fa3b8",
                5_090_249,
                "https://files.pythonhosted.org/packages/bd/a7/2332679479c70b68dccbf4a8eb9c9b5ee383164b161bee9284ac141fbd33/numpy-2.1.3-cp312-cp312-macosx_14_0_arm64.whl",
            ),
            (
                "pyarrow-18.1.0-cp312-cp312-macosx_12_0_arm64.whl",
                "9f3a76670b263dc41d0ae877f09124ab96ce10e4e48f3e3e4257273cee61ad0d",
                29_514_620,
                "https://files.pythonhosted.org/packages/6a/50/12829e7111b932581e51dda51d5cb39207a056c30fe31ef43f14c63c4d7e/pyarrow-18.1.0-cp312-cp312-macosx_12_0_arm64.whl",
            ),
        ),
        RuntimePlatform::WindowsX86_64 => (
            (
                "numpy-2.1.3-cp312-cp312-win_amd64.whl",
                "0d30c543f02e84e92c4b1f415b7c6b5326cbe45ee7882b6b77db7195fb971e3a",
                12_566_858,
                "https://files.pythonhosted.org/packages/a6/84/fa11dad3404b7634aaab50733581ce11e5350383311ea7a7010f464c0170/numpy-2.1.3-cp312-cp312-win_amd64.whl",
            ),
            (
                "pyarrow-18.1.0-cp312-cp312-win_amd64.whl",
                "0ad4892617e1a6c7a551cfc827e072a633eaff758fa09f21c4ee548c30bcaf99",
                25_092_330,
                "https://files.pythonhosted.org/packages/76/52/f8da04195000099d394012b8d42c503d7041b79f778d854f410e5f05049a/pyarrow-18.1.0-cp312-cp312-win_amd64.whl",
            ),
        ),
        RuntimePlatform::LinuxX86_64 => (
            (
                "numpy-2.1.3-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
                "2312b2aa89e1f43ecea6da6ea9a810d06aae08321609d8dc0d0eda6d946a541b",
                16_043_185,
                "https://files.pythonhosted.org/packages/9e/3e/3757f304c704f2f0294a6b8340fcf2be244038be07da4cccf390fa678a9f/numpy-2.1.3-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
            ),
            (
                "pyarrow-18.1.0-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
                "0743e503c55be0fdb5c08e7d44853da27f19dc854531c0570f9f394ec9671d54",
                40_139_341,
                "https://files.pythonhosted.org/packages/6e/f6/19360dae44200e35753c5c2889dc478154cd78e61b1f738514c9f131734d/pyarrow-18.1.0-cp312-cp312-manylinux_2_17_x86_64.manylinux2014_x86_64.whl",
            ),
        ),
    };
    let mut wheels = vec![sdk, runner, qlib_adapter];
    let mut download_urls = BTreeMap::new();
    for (file_name, package, version, sha256, size, url) in [
        (numpy.0, "numpy", "2.1.3", numpy.1, numpy.2, numpy.3),
        (arrow.0, "pyarrow", "18.1.0", arrow.1, arrow.2, arrow.3),
    ] {
        wheels.push(WheelIdentity {
            file_name: file_name.into(),
            package: package.into(),
            version: version.into(),
            sha256: sha256.into(),
            size,
            platform_tags: vec![platform.tag().into()],
        });
        download_urls.insert(file_name.into(), url.into());
    }
    let mut manifest = WheelhouseManifest {
        schema: WHEELHOUSE_MANIFEST_SCHEMA.into(),
        identity: String::new(),
        runtime_profile: PYTHON_RUNTIME_PROFILE.into(),
        platform,
        wheels,
        signature: String::new(),
    };
    manifest.identity = wheelhouse_identity(&manifest.wheels)?;
    manifest.signature = manifest.expected_signature()?;
    Ok(WheelhouseCatalogEntry {
        manifest,
        download_urls,
    })
}

pub fn embedded_wheel_payload(file_name: &str) -> Option<&'static [u8]> {
    match file_name {
        "adaq_research_sdk-1.0.0-py3-none-any.whl" => Some(include_bytes!(
            "../resources/wheels/adaq_research_sdk-1.0.0-py3-none-any.whl"
        )),
        "adaq_python_research_runner-1.0.0-py3-none-any.whl" => Some(include_bytes!(
            "../resources/wheels/adaq_python_research_runner-1.0.0-py3-none-any.whl"
        )),
        "adaq_qlib_ridge_adapter-1.0.0-py3-none-any.whl" => Some(include_bytes!(
            "../resources/wheels/adaq_qlib_ridge_adapter-1.0.0-py3-none-any.whl"
        )),
        _ => None,
    }
}

impl RuntimePlatform {
    pub fn current() -> Result<Self, PythonResearchError> {
        match (std::env::consts::OS, std::env::consts::ARCH) {
            ("macos", "aarch64") => Ok(Self::MacosAarch64),
            ("windows", "x86_64") => Ok(Self::WindowsX86_64),
            ("linux", "x86_64") => Ok(Self::LinuxX86_64),
            _ => Err(invalid("python-runtime-platform-unsupported")),
        }
    }

    pub fn tag(self) -> &'static str {
        match self {
            Self::MacosAarch64 => "macos-aarch64",
            Self::WindowsX86_64 => "windows-x86_64",
            Self::LinuxX86_64 => "linux-x86_64",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeArtifactManifest {
    pub schema: String,
    pub profile: String,
    pub version: String,
    pub platform: RuntimePlatform,
    pub source: String,
    pub license: String,
    pub file_name: String,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub artifact_sha256: String,
    pub signature: String,
}

impl RuntimeArtifactManifest {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, PythonResearchError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Unsigned<'a> {
            schema: &'a str,
            profile: &'a str,
            version: &'a str,
            platform: RuntimePlatform,
            source: &'a str,
            license: &'a str,
            file_name: &'a str,
            download_bytes: u64,
            installed_bytes: u64,
            artifact_sha256: &'a str,
        }
        serde_json::to_vec(&Unsigned {
            schema: &self.schema,
            profile: &self.profile,
            version: &self.version,
            platform: self.platform,
            source: &self.source,
            license: &self.license,
            file_name: &self.file_name,
            download_bytes: self.download_bytes,
            installed_bytes: self.installed_bytes,
            artifact_sha256: &self.artifact_sha256,
        })
        .map_err(|error| invalid(error.to_string()))
    }

    pub fn expected_signature(&self) -> Result<String, PythonResearchError> {
        let mut mac = HmacSha256::new_from_slice(TRUSTED_RUNTIME_SIGNING_KEY)
            .map_err(|_| invalid("python-runtime-signing-key-invalid"))?;
        mac.update(&self.signing_bytes()?);
        Ok(hex_bytes(&mac.finalize().into_bytes()))
    }

    pub fn validate(&self, payload: &[u8]) -> Result<(), PythonResearchError> {
        if self.schema != RUNTIME_MANIFEST_SCHEMA
            || self.profile != PYTHON_RUNTIME_PROFILE
            || !self.version.starts_with("3.12.")
            || self.source.trim().is_empty()
            || self.license.trim().is_empty()
            || self.file_name.contains('/')
            || self.file_name.contains('\\')
            || self.download_bytes != payload.len() as u64
            || self.installed_bytes == 0
            || !is_sha256(&self.artifact_sha256)
            || sha256(payload) != self.artifact_sha256
            || self.signature != self.expected_signature()?
        {
            return Err(invalid("python-runtime-manifest-invalid"));
        }
        if self.installed_bytes != runtime_archive_installed_bytes(payload)? {
            return Err(invalid("python-runtime-installed-size-mismatch"));
        }
        validate_runtime_archive(payload)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PreparationStatus {
    Missing,
    Preparing,
    Ready,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PreparationAttempt {
    pub attempt_id: String,
    pub user_id: String,
    pub status: PreparationStatus,
    pub identity: Option<String>,
    pub source_attempt_id: Option<String>,
    pub diagnostic: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RuntimeRecord {
    pub artifact_sha256: String,
    pub profile: String,
    pub version: String,
    pub platform: RuntimePlatform,
    pub path: String,
    pub download_bytes: u64,
    pub installed_bytes: u64,
    pub source: String,
    pub license: String,
    pub file_name: String,
    pub signature: String,
}

#[derive(Debug, Clone)]
pub struct RuntimeStore {
    root: PathBuf,
    gate: Arc<Mutex<()>>,
    attempts_path: PathBuf,
    attempts: Arc<Mutex<PreparationDatabase>>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct PreparationDatabase {
    attempts: BTreeMap<String, PreparationAttempt>,
}

impl RuntimeStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        let root = root.into();
        let attempts_path = root.join("runtime-preparations.json");
        let mut attempts: PreparationDatabase = fs::read(&attempts_path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        let mut recovered = false;
        for attempt in attempts.attempts.values_mut() {
            if attempt.status == PreparationStatus::Preparing {
                attempt.status = PreparationStatus::Failed;
                attempt.diagnostic = Some("python-runtime-preparation-interrupted".into());
                recovered = true;
            }
        }
        if recovered {
            let _ = persist_json(&attempts_path, &attempts);
        }
        if let Ok(entries) = fs::read_dir(root.join("runtimes")) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name.starts_with('.') && name.contains(".staging-") {
                    let _ = fs::remove_dir_all(entry.path());
                }
            }
        }
        Self {
            root,
            gate: Arc::new(Mutex::new(())),
            attempts_path,
            attempts: Arc::new(Mutex::new(attempts)),
        }
    }

    pub fn prepare(
        &self,
        user_id: &str,
        manifest: &RuntimeArtifactManifest,
        payload: &[u8],
        cancelled: impl Fn() -> bool,
    ) -> Result<(PreparationAttempt, Option<RuntimeRecord>), PythonResearchError> {
        self.prepare_with_source(user_id, None, manifest, payload, cancelled)
    }

    pub fn prepare_with_source(
        &self,
        user_id: &str,
        source_attempt_id: Option<String>,
        manifest: &RuntimeArtifactManifest,
        payload: &[u8],
        cancelled: impl Fn() -> bool,
    ) -> Result<(PreparationAttempt, Option<RuntimeRecord>), PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-runtime-store-lock-poisoned"))?;
        crate::validate_user_id(user_id)?;
        self.validate_source_attempt(user_id, source_attempt_id.as_deref())?;
        let attempt_id = format!(
            "runtime-{}-{}",
            unique_suffix(),
            sha256(manifest.artifact_sha256.as_bytes())
        );
        self.save_attempt(PreparationAttempt {
            attempt_id: attempt_id.clone(),
            user_id: user_id.into(),
            status: PreparationStatus::Preparing,
            identity: Some(manifest.artifact_sha256.clone()),
            source_attempt_id,
            diagnostic: None,
        })?;
        if cancelled() {
            let attempt = self.finish_attempt(
                &attempt_id,
                PreparationStatus::Cancelled,
                Some("python-runtime-preparation-cancelled".into()),
            )?;
            return Ok((attempt, None));
        }
        let current_platform = match RuntimePlatform::current() {
            Ok(platform) => platform,
            Err(error) => {
                let _ = self.finish_attempt(
                    &attempt_id,
                    PreparationStatus::Failed,
                    Some(error.0.clone()),
                );
                return Err(error);
            }
        };
        if manifest.platform != current_platform {
            let error = invalid("python-runtime-platform-mismatch");
            let _ = self.finish_attempt(
                &attempt_id,
                PreparationStatus::Failed,
                Some(error.0.clone()),
            );
            return Err(error);
        }
        if let Err(error) = manifest.validate(payload) {
            let _ = self.finish_attempt(
                &attempt_id,
                PreparationStatus::Failed,
                Some(error.0.clone()),
            );
            return Err(error);
        }
        let destination = self.root.join("runtimes").join(&manifest.artifact_sha256);
        if runtime_cache_matches(&destination, manifest) {
            let attempt = self.finish_attempt(&attempt_id, PreparationStatus::Ready, None)?;
            return Ok((attempt, Some(runtime_record(manifest, &destination))));
        }
        let staging = staging_path(&destination)?;
        let result = (|| {
            fs::create_dir_all(&staging)?;
            extract_runtime_archive(payload, &staging)?;
            fs::write(
                staging.join("adaq-runtime-manifest.json"),
                serde_json::to_vec(manifest).map_err(|error| invalid(error.to_string()))?,
            )?;
            if cancelled() {
                return Err(invalid("python-runtime-preparation-cancelled"));
            }
            replace_directory(&staging, &destination)?;
            Ok::<(), PythonResearchError>(())
        })();
        if result.is_err() {
            fs::remove_dir_all(&staging).ok();
        }
        match result {
            Ok(()) => {
                let attempt = self.finish_attempt(&attempt_id, PreparationStatus::Ready, None)?;
                Ok((attempt, Some(runtime_record(manifest, &destination))))
            }
            Err(error) if error.0 == "python-runtime-preparation-cancelled" => {
                let attempt =
                    self.finish_attempt(&attempt_id, PreparationStatus::Cancelled, Some(error.0))?;
                Ok((attempt, None))
            }
            Err(error) => {
                let diagnostic = error.0.clone();
                let _ =
                    self.finish_attempt(&attempt_id, PreparationStatus::Failed, Some(diagnostic));
                Err(error)
            }
        }
    }

    pub fn record_cancelled(
        &self,
        user_id: &str,
        source_attempt_id: Option<String>,
        identity: Option<String>,
    ) -> Result<PreparationAttempt, PythonResearchError> {
        self.record_terminal(
            user_id,
            source_attempt_id,
            identity,
            PreparationStatus::Cancelled,
            Some("python-runtime-preparation-cancelled".into()),
        )
    }

    pub fn record_failed(
        &self,
        user_id: &str,
        source_attempt_id: Option<String>,
        identity: Option<String>,
        diagnostic: String,
    ) -> Result<PreparationAttempt, PythonResearchError> {
        self.record_terminal(
            user_id,
            source_attempt_id,
            identity,
            PreparationStatus::Failed,
            Some(diagnostic),
        )
    }

    pub fn record_ready(
        &self,
        user_id: &str,
        source_attempt_id: Option<String>,
        identity: String,
    ) -> Result<PreparationAttempt, PythonResearchError> {
        self.record_terminal(
            user_id,
            source_attempt_id,
            Some(identity),
            PreparationStatus::Ready,
            None,
        )
    }

    pub fn cached_record(&self, manifest: &RuntimeArtifactManifest) -> Option<RuntimeRecord> {
        self.ready_record(manifest).ok()
    }

    pub fn ready_record(
        &self,
        manifest: &RuntimeArtifactManifest,
    ) -> Result<RuntimeRecord, PythonResearchError> {
        let destination = self.root.join("runtimes").join(&manifest.artifact_sha256);
        if !runtime_cache_matches(&destination, manifest) {
            return Err(invalid("python-runtime-cache-invalid"));
        }
        self.executable_path(&manifest.artifact_sha256)?;
        Ok(runtime_record(manifest, &destination))
    }

    fn record_terminal(
        &self,
        user_id: &str,
        source_attempt_id: Option<String>,
        identity: Option<String>,
        status: PreparationStatus,
        diagnostic: Option<String>,
    ) -> Result<PreparationAttempt, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-runtime-store-lock-poisoned"))?;
        crate::validate_user_id(user_id)?;
        self.validate_source_attempt(user_id, source_attempt_id.as_deref())?;
        if identity
            .as_ref()
            .is_some_and(|identity| !is_sha256(identity))
        {
            return Err(invalid("python-runtime-identity-invalid"));
        }
        let attempt = PreparationAttempt {
            attempt_id: format!(
                "runtime-{}-{}",
                unique_suffix(),
                identity.as_deref().unwrap_or("unknown")
            ),
            user_id: user_id.into(),
            status,
            identity,
            source_attempt_id,
            diagnostic,
        };
        self.save_attempt(attempt.clone())?;
        Ok(attempt)
    }

    fn validate_source_attempt(
        &self,
        user_id: &str,
        source_attempt_id: Option<&str>,
    ) -> Result<(), PythonResearchError> {
        let Some(source_attempt_id) = source_attempt_id else {
            return Ok(());
        };
        let database = self
            .attempts
            .lock()
            .map_err(|_| invalid("python-runtime-attempt-store-lock-poisoned"))?;
        let source = database
            .attempts
            .get(source_attempt_id)
            .ok_or_else(|| invalid("python-runtime-source-attempt-not-found"))?;
        if source.user_id != user_id {
            return Err(invalid("python-runtime-source-attempt-user-mismatch"));
        }
        Ok(())
    }

    pub fn list_preparations(
        &self,
        user_id: &str,
    ) -> Result<Vec<PreparationAttempt>, PythonResearchError> {
        crate::validate_user_id(user_id)?;
        let mut attempts = self
            .attempts
            .lock()
            .map_err(|_| invalid("python-runtime-attempt-store-lock-poisoned"))?
            .attempts
            .values()
            .filter(|attempt| attempt.user_id == user_id)
            .cloned()
            .collect::<Vec<_>>();
        attempts.sort_by(|left, right| left.attempt_id.cmp(&right.attempt_id));
        Ok(attempts)
    }

    pub fn latest_preparation(&self) -> Result<Option<PreparationAttempt>, PythonResearchError> {
        Ok(self
            .attempts
            .lock()
            .map_err(|_| invalid("python-runtime-attempt-store-lock-poisoned"))?
            .attempts
            .values()
            .max_by(|left, right| left.attempt_id.cmp(&right.attempt_id))
            .cloned())
    }

    fn save_attempt(&self, attempt: PreparationAttempt) -> Result<(), PythonResearchError> {
        let mut database = self
            .attempts
            .lock()
            .map_err(|_| invalid("python-runtime-attempt-store-lock-poisoned"))?;
        database
            .attempts
            .insert(attempt.attempt_id.clone(), attempt);
        persist_json(&self.attempts_path, &*database)
    }

    fn finish_attempt(
        &self,
        attempt_id: &str,
        status: PreparationStatus,
        diagnostic: Option<String>,
    ) -> Result<PreparationAttempt, PythonResearchError> {
        let mut database = self
            .attempts
            .lock()
            .map_err(|_| invalid("python-runtime-attempt-store-lock-poisoned"))?;
        let attempt = database
            .attempts
            .get_mut(attempt_id)
            .ok_or_else(|| invalid("python-runtime-attempt-not-found"))?;
        attempt.status = status;
        attempt.diagnostic = diagnostic;
        let result = attempt.clone();
        persist_json(&self.attempts_path, &*database)?;
        Ok(result)
    }

    pub fn evict_inactive(
        &self,
        active_artifacts: &BTreeSet<String>,
    ) -> Result<Vec<String>, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-runtime-store-lock-poisoned"))?;
        let directory = self.root.join("runtimes");
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut removed = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !active_artifacts.contains(&name) && entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
                removed.push(name);
            }
        }
        Ok(removed)
    }

    pub fn executable_path(&self, artifact_sha256: &str) -> Result<PathBuf, PythonResearchError> {
        if !is_sha256(artifact_sha256) {
            return Err(invalid("python-runtime-identity-invalid"));
        }
        let directory = self.root.join("runtimes").join(artifact_sha256);
        if directory.join("adaq-runtime-disabled").is_file() {
            return Err(invalid("python-runtime-security-disabled"));
        }
        [
            directory.join("runtime/python"),
            directory.join("runtime/python.exe"),
            directory.join("python/bin/python3.12"),
            directory.join("python/python.exe"),
        ]
        .into_iter()
        .find(|path| {
            path.is_file() && {
                #[cfg(unix)]
                {
                    fs::metadata(path)
                        .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                        .unwrap_or(false)
                }
                #[cfg(not(unix))]
                {
                    true
                }
            }
        })
        .ok_or_else(|| invalid("python-runtime-executable-not-ready"))
    }

    pub fn disable_execution(&self, artifact_sha256: &str) -> Result<(), PythonResearchError> {
        if !is_sha256(artifact_sha256) {
            return Err(invalid("python-runtime-identity-invalid"));
        }
        let directory = self.root.join("runtimes").join(artifact_sha256);
        if !directory.is_dir() {
            return Err(invalid("python-runtime-not-found"));
        }
        fs::write(
            directory.join("adaq-runtime-disabled"),
            b"security-disabled",
        )?;
        Ok(())
    }
}

fn runtime_record(manifest: &RuntimeArtifactManifest, destination: &Path) -> RuntimeRecord {
    RuntimeRecord {
        artifact_sha256: manifest.artifact_sha256.clone(),
        profile: manifest.profile.clone(),
        version: manifest.version.clone(),
        platform: manifest.platform,
        path: destination.to_string_lossy().into_owned(),
        download_bytes: manifest.download_bytes,
        installed_bytes: manifest.installed_bytes,
        source: manifest.source.clone(),
        license: manifest.license.clone(),
        file_name: manifest.file_name.clone(),
        signature: manifest.signature.clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WheelIdentity {
    pub file_name: String,
    pub package: String,
    pub version: String,
    pub sha256: String,
    pub size: u64,
    pub platform_tags: Vec<String>,
}

impl WheelIdentity {
    fn validate_metadata(&self) -> Result<(), PythonResearchError> {
        validate_wheel_filename(&self.file_name)?;
        let parts = self
            .file_name
            .strip_suffix(".whl")
            .unwrap_or_default()
            .split('-')
            .collect::<Vec<_>>();
        if self.package.trim().is_empty()
            || self.version.trim().is_empty()
            || !is_sha256(&self.sha256)
            || self.platform_tags.is_empty()
            || self.platform_tags.iter().any(|tag| tag.trim().is_empty())
            || parts.get(1).copied() != Some(self.version.as_str())
        {
            return Err(invalid("python-wheel-identity-invalid"));
        }
        let file_package = self
            .file_name
            .split('-')
            .next()
            .unwrap_or_default()
            .replace('_', "-")
            .to_lowercase();
        if file_package != self.package.replace('_', "-").to_lowercase() {
            return Err(invalid("python-wheel-identity-invalid"));
        }
        Ok(())
    }

    pub fn validate(
        &self,
        payload: &[u8],
        platform: RuntimePlatform,
    ) -> Result<(), PythonResearchError> {
        self.validate_metadata()?;
        if !self.file_name.ends_with(".whl")
            || self.file_name.contains('/')
            || self.file_name.contains('\\')
            || self.size != payload.len() as u64
            || sha256(payload) != self.sha256
            || !self
                .platform_tags
                .iter()
                .any(|tag| tag == "any" || tag == platform.tag())
        {
            return Err(invalid("python-wheel-identity-invalid"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WheelhouseManifest {
    pub schema: String,
    pub identity: String,
    pub runtime_profile: String,
    pub platform: RuntimePlatform,
    pub wheels: Vec<WheelIdentity>,
    pub signature: String,
}

impl WheelhouseManifest {
    pub fn signing_bytes(&self) -> Result<Vec<u8>, PythonResearchError> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Unsigned<'a> {
            schema: &'a str,
            identity: &'a str,
            runtime_profile: &'a str,
            platform: RuntimePlatform,
            wheels: &'a [WheelIdentity],
        }
        serde_json::to_vec(&Unsigned {
            schema: &self.schema,
            identity: &self.identity,
            runtime_profile: &self.runtime_profile,
            platform: self.platform,
            wheels: &self.wheels,
        })
        .map_err(|error| invalid(error.to_string()))
    }

    pub fn expected_signature(&self) -> Result<String, PythonResearchError> {
        let mut mac = HmacSha256::new_from_slice(TRUSTED_RUNTIME_SIGNING_KEY)
            .map_err(|_| invalid("python-wheelhouse-signing-key-invalid"))?;
        mac.update(&self.signing_bytes()?);
        Ok(hex_bytes(&mac.finalize().into_bytes()))
    }

    pub fn validate(
        &self,
        payloads: &BTreeMap<String, Vec<u8>>,
    ) -> Result<(), PythonResearchError> {
        if self.schema != WHEELHOUSE_MANIFEST_SCHEMA
            || self.runtime_profile != PYTHON_RUNTIME_PROFILE
            || self.wheels.is_empty()
            || self.signature != self.expected_signature()?
            || !is_sha256(&self.identity)
            || self.identity != wheelhouse_identity(&self.wheels)?
        {
            return Err(invalid("python-wheelhouse-manifest-invalid"));
        }
        let names = self
            .wheels
            .iter()
            .map(|wheel| wheel.package.as_str())
            .collect::<BTreeSet<_>>();
        if names.len() != self.wheels.len()
            || self
                .wheels
                .iter()
                .map(|wheel| wheel.file_name.as_str())
                .collect::<BTreeSet<_>>()
                .len()
                != self.wheels.len()
        {
            return Err(invalid("python-wheelhouse-duplicate-wheel"));
        }
        if REQUIRED_WHEEL_PACKAGES
            .iter()
            .any(|package| !names.contains(package))
        {
            return Err(invalid("python-wheelhouse-required-package-missing"));
        }
        for wheel in &self.wheels {
            wheel.validate_metadata()?;
            let payload = payloads
                .get(&wheel.file_name)
                .ok_or_else(|| invalid("python-wheelhouse-wheel-missing"))?;
            wheel.validate(payload, self.platform)?;
        }
        if payloads
            .keys()
            .any(|name| !self.wheels.iter().any(|wheel| &wheel.file_name == name))
        {
            return Err(invalid("python-wheelhouse-contains-undeclared-wheel"));
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct WheelhouseRecord {
    pub identity: String,
    pub path: String,
    pub wheel_count: usize,
    pub disk_bytes: u64,
}

#[derive(Debug, Clone)]
pub struct WheelhouseStore {
    root: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl WheelhouseStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub fn prepare(
        &self,
        manifest: &WheelhouseManifest,
        payloads: &BTreeMap<String, Vec<u8>>,
    ) -> Result<WheelhouseRecord, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-wheelhouse-store-lock-poisoned"))?;
        manifest.validate(payloads)?;
        let destination = self.root.join("wheelhouses").join(&manifest.identity);
        if self.cache_matches(&destination, manifest) {
            return Ok(wheelhouse_record(manifest, &destination));
        }
        let staging = staging_path(&destination)?;
        let result = (|| {
            fs::create_dir_all(staging.join("wheels"))?;
            fs::write(
                staging.join("adaq-wheelhouse-manifest.json"),
                serde_json::to_vec(manifest).map_err(|error| invalid(error.to_string()))?,
            )?;
            for wheel in &manifest.wheels {
                let payload = payloads
                    .get(&wheel.file_name)
                    .ok_or_else(|| invalid("python-wheelhouse-wheel-missing"))?;
                fs::write(staging.join("wheels").join(&wheel.file_name), payload)?;
            }
            replace_directory(&staging, &destination)?;
            Ok::<(), PythonResearchError>(())
        })();
        if result.is_err() {
            fs::remove_dir_all(&staging).ok();
        }
        result?;
        Ok(wheelhouse_record(manifest, &destination))
    }

    pub fn load(
        &self,
        identity: &str,
    ) -> Result<(WheelhouseManifest, BTreeMap<String, Vec<u8>>), PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-wheelhouse-store-lock-poisoned"))?;
        if !is_sha256(identity) {
            return Err(invalid("python-wheelhouse-identity-invalid"));
        }
        let destination = self.root.join("wheelhouses").join(identity);
        let manifest: WheelhouseManifest = serde_json::from_slice(&fs::read(
            destination.join("adaq-wheelhouse-manifest.json"),
        )?)
        .map_err(|error| invalid(format!("python-wheelhouse-manifest-unreadable:{error}")))?;
        if manifest.identity != identity {
            return Err(invalid("python-wheelhouse-identity-mismatch"));
        }
        let mut payloads = BTreeMap::new();
        for wheel in &manifest.wheels {
            payloads.insert(
                wheel.file_name.clone(),
                fs::read(destination.join("wheels").join(&wheel.file_name))?,
            );
        }
        manifest.validate(&payloads)?;
        Ok((manifest, payloads))
    }

    pub fn evict_inactive(
        &self,
        active_identities: &BTreeSet<String>,
    ) -> Result<Vec<String>, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-wheelhouse-store-lock-poisoned"))?;
        let directory = self.root.join("wheelhouses");
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut removed = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !active_identities.contains(&name) && entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
                removed.push(name);
            }
        }
        Ok(removed)
    }

    pub fn disk_bytes(&self, identity: &str) -> u64 {
        if !is_sha256(identity) {
            return 0;
        }
        directory_bytes(&self.root.join("wheelhouses").join(identity))
    }

    fn cache_matches(&self, destination: &Path, manifest: &WheelhouseManifest) -> bool {
        if !destination.is_dir() {
            return false;
        }
        let stored: WheelhouseManifest =
            match fs::read(destination.join("adaq-wheelhouse-manifest.json"))
                .ok()
                .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            {
                Some(stored) => stored,
                None => return false,
            };
        if stored != *manifest {
            return false;
        }
        manifest.wheels.iter().all(|wheel| {
            fs::read(destination.join("wheels").join(&wheel.file_name)).is_ok_and(|payload| {
                payload.len() as u64 == wheel.size && sha256(&payload) == wheel.sha256
            })
        })
    }
}

fn wheelhouse_record(manifest: &WheelhouseManifest, destination: &Path) -> WheelhouseRecord {
    WheelhouseRecord {
        identity: manifest.identity.clone(),
        path: destination.to_string_lossy().into_owned(),
        wheel_count: manifest.wheels.len(),
        disk_bytes: directory_bytes(destination),
    }
}

pub fn wheelhouse_identity(wheels: &[WheelIdentity]) -> Result<String, PythonResearchError> {
    let mut ordered = wheels.to_vec();
    ordered.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    serde_json::to_vec(&ordered)
        .map(|bytes| sha256(&bytes))
        .map_err(|error| invalid(error.to_string()))
}

pub fn validate_wheel_filename(file_name: &str) -> Result<(), PythonResearchError> {
    if !file_name.ends_with(".whl")
        || file_name.contains('/')
        || file_name.contains('\\')
        || file_name.split('-').count() < 5
    {
        return Err(invalid("python-source-distribution-or-wheel-name-invalid"));
    }
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DependencyIntent {
    pub project_name: String,
    pub dependencies: Vec<String>,
}

impl DependencyIntent {
    pub fn parse(pyproject: &[u8]) -> Result<Self, PythonResearchError> {
        let value: toml::Value = toml::from_str(
            std::str::from_utf8(pyproject)
                .map_err(|_| invalid("python-dependency-intent-must-be-utf8"))?,
        )
        .map_err(|error| invalid(format!("invalid-python-dependency-intent:{error}")))?;
        let project = value
            .get("project")
            .and_then(toml::Value::as_table)
            .ok_or_else(|| invalid("python-dependency-intent-project-missing"))?;
        let project_name = project
            .get("name")
            .and_then(toml::Value::as_str)
            .ok_or_else(|| invalid("python-dependency-intent-name-missing"))?;
        if project_name.trim().is_empty() {
            return Err(invalid("python-dependency-intent-name-empty"));
        }
        let dependencies = project
            .get("dependencies")
            .and_then(toml::Value::as_array)
            .map(|values| {
                values
                    .iter()
                    .map(|value| {
                        value
                            .as_str()
                            .map(str::to_owned)
                            .ok_or_else(|| invalid("python-dependency-intent-entry-invalid"))
                    })
                    .collect::<Result<Vec<_>, PythonResearchError>>()
            })
            .transpose()?
            .unwrap_or_default();
        for dependency in &dependencies {
            let (name, version) = dependency.split_once("==").unwrap_or(("", ""));
            if name.trim().is_empty()
                || version.trim().is_empty()
                || name.contains('[')
                || dependency.contains(';')
                || dependency.contains("://")
                || dependency.contains("git+")
                || dependency.contains(" @ ")
                || dependency.ends_with(".tar.gz")
            {
                return Err(invalid("python-dependency-intent-source-or-vcs-rejected"));
            }
        }
        Ok(Self {
            project_name: project_name.into(),
            dependencies,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentLock {
    pub schema: String,
    pub runtime_artifact_sha256: String,
    pub wheelhouse_identity: String,
    pub platform: RuntimePlatform,
    pub wheels: Vec<WheelIdentity>,
    pub lock_sha256: String,
}

impl EnvironmentLock {
    pub fn validate(&self) -> Result<(), PythonResearchError> {
        if self.schema != ENVIRONMENT_LOCK_SCHEMA
            || !is_sha256(&self.runtime_artifact_sha256)
            || !is_sha256(&self.wheelhouse_identity)
            || self.wheels.is_empty()
            || !is_sha256(&self.lock_sha256)
        {
            return Err(invalid("python-environment-lock-invalid"));
        }
        let content = lock_content(self)?;
        if self.lock_sha256 != sha256(&content) {
            return Err(invalid("python-environment-lock-hash-mismatch"));
        }
        let mut names = BTreeSet::new();
        let mut files = BTreeSet::new();
        for wheel in &self.wheels {
            wheel.validate_metadata()?;
            if !names.insert(wheel.package.as_str()) || !files.insert(wheel.file_name.as_str()) {
                return Err(invalid("python-environment-lock-duplicate-wheel"));
            }
        }
        Ok(())
    }
}

pub fn parse_environment_lock(bytes: &[u8]) -> Result<EnvironmentLock, PythonResearchError> {
    let lock: EnvironmentLock = toml::from_str(
        std::str::from_utf8(bytes)
            .map_err(|_| invalid("python-environment-lock-unreadable:utf8"))?,
    )
    .map_err(|error| invalid(format!("python-environment-lock-unreadable:{error}")))?;
    lock.validate()?;
    Ok(lock)
}

pub fn sync_environment(
    runtime_artifact_sha256: &str,
    platform: RuntimePlatform,
    intent: &DependencyIntent,
    wheelhouse: &WheelhouseManifest,
    payloads: &BTreeMap<String, Vec<u8>>,
) -> Result<EnvironmentLock, PythonResearchError> {
    if !is_sha256(runtime_artifact_sha256) || wheelhouse.platform != platform {
        return Err(invalid("python-environment-input-identity-invalid"));
    }
    wheelhouse.validate(payloads)?;
    let mut wheels = wheelhouse.wheels.clone();
    for dependency in &intent.dependencies {
        let (name, requested_version) = dependency
            .split_once("==")
            .ok_or_else(|| invalid("python-dependency-version-missing"))?;
        let package = name.trim().replace('-', "_").to_lowercase();
        if !wheels.iter().any(|wheel| {
            wheel.package.replace('-', "_").to_lowercase() == package
                && wheel.version == requested_version.trim()
        }) {
            return Err(invalid(
                "python-dependency-not-present-in-trusted-wheelhouse",
            ));
        }
        if dependency.contains('>')
            || dependency.contains('<')
            || dependency.contains('~')
            || dependency.contains('*')
        {
            return Err(invalid("python-dependency-version-range-not-reproducible"));
        }
    }
    wheels.sort_by(|left, right| left.file_name.cmp(&right.file_name));
    let mut lock = EnvironmentLock {
        schema: ENVIRONMENT_LOCK_SCHEMA.into(),
        runtime_artifact_sha256: runtime_artifact_sha256.into(),
        wheelhouse_identity: wheelhouse.identity.clone(),
        platform,
        wheels,
        lock_sha256: String::new(),
    };
    lock.lock_sha256 = sha256(&lock_content(&lock)?);
    lock.validate()?;
    Ok(lock)
}

fn lock_content(lock: &EnvironmentLock) -> Result<Vec<u8>, PythonResearchError> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Content<'a> {
        schema: &'a str,
        runtime_artifact_sha256: &'a str,
        wheelhouse_identity: &'a str,
        platform: RuntimePlatform,
        wheels: &'a [WheelIdentity],
    }
    serde_json::to_vec(&Content {
        schema: &lock.schema,
        runtime_artifact_sha256: &lock.runtime_artifact_sha256,
        wheelhouse_identity: &lock.wheelhouse_identity,
        platform: lock.platform,
        wheels: &lock.wheels,
    })
    .map_err(|error| invalid(error.to_string()))
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct EnvironmentRecord {
    pub environment_sha256: String,
    pub path: String,
    pub lock_sha256: String,
    pub wheel_count: usize,
}

#[derive(Debug, Clone)]
pub struct EnvironmentStore {
    root: PathBuf,
    gate: Arc<Mutex<()>>,
}

impl EnvironmentStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self {
            root: root.into(),
            gate: Arc::new(Mutex::new(())),
        }
    }

    pub fn prepare(
        &self,
        lock: &EnvironmentLock,
        payloads: &BTreeMap<String, Vec<u8>>,
        wheelhouse: &WheelhouseManifest,
    ) -> Result<EnvironmentRecord, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-environment-store-lock-poisoned"))?;
        lock.validate()?;
        wheelhouse.validate(payloads)?;
        if lock.wheelhouse_identity != wheelhouse.identity
            || lock.platform != wheelhouse.platform
            || lock.wheels.len() != wheelhouse.wheels.len()
            || lock
                .wheels
                .iter()
                .any(|wheel| !wheelhouse.wheels.contains(wheel))
        {
            return Err(invalid("python-environment-wheelhouse-mismatch"));
        }
        let environment_sha256 =
            sha256(&serde_json::to_vec(lock).map_err(|error| invalid(error.to_string()))?);
        let destination = self.root.join("environments").join(&environment_sha256);
        if self.environment_cache_matches(&destination, lock) {
            return Ok(EnvironmentRecord {
                environment_sha256,
                path: destination.to_string_lossy().into_owned(),
                lock_sha256: lock.lock_sha256.clone(),
                wheel_count: lock.wheels.len(),
            });
        }
        let staging = staging_path(&destination)?;
        let result = (|| {
            fs::create_dir_all(staging.join("wheels"))?;
            fs::write(
                staging.join("pylock.toml"),
                toml::to_string(lock)?.into_bytes(),
            )?;
            for wheel in &lock.wheels {
                let payload = payloads
                    .get(&wheel.file_name)
                    .ok_or_else(|| invalid("python-environment-wheel-missing"))?;
                fs::write(staging.join("wheels").join(&wheel.file_name), payload)?;
            }
            replace_directory(&staging, &destination)?;
            Ok::<(), PythonResearchError>(())
        })();
        if result.is_err() {
            fs::remove_dir_all(&staging).ok();
        }
        result?;
        Ok(EnvironmentRecord {
            environment_sha256,
            path: destination.to_string_lossy().into_owned(),
            lock_sha256: lock.lock_sha256.clone(),
            wheel_count: lock.wheels.len(),
        })
    }

    pub fn evict_inactive(
        &self,
        active_environments: &BTreeSet<String>,
    ) -> Result<Vec<String>, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-environment-store-lock-poisoned"))?;
        let directory = self.root.join("environments");
        if !directory.is_dir() {
            return Ok(Vec::new());
        }
        let mut removed = Vec::new();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if !active_environments.contains(&name) && entry.file_type()?.is_dir() {
                fs::remove_dir_all(entry.path())?;
                removed.push(name);
            }
        }
        Ok(removed)
    }

    fn environment_cache_matches(&self, destination: &Path, lock: &EnvironmentLock) -> bool {
        if !destination.is_dir() {
            return false;
        }
        let lock_bytes = match fs::read(destination.join("pylock.toml")) {
            Ok(bytes) => bytes,
            Err(_) => return false,
        };
        let stored_lock = match std::str::from_utf8(&lock_bytes)
            .ok()
            .and_then(|text| toml::from_str::<EnvironmentLock>(text).ok())
        {
            Some(lock) => lock,
            None => return false,
        };
        stored_lock == *lock
            && lock.wheels.iter().all(|wheel| {
                let path = destination.join("wheels").join(&wheel.file_name);
                fs::read(path).is_ok_and(|bytes| {
                    bytes.len() as u64 == wheel.size && sha256(&bytes) == wheel.sha256
                })
            })
    }

    pub fn wheel_path(
        &self,
        environment_sha256: &str,
        package: &str,
    ) -> Result<PathBuf, PythonResearchError> {
        if !is_sha256(environment_sha256) || package.trim().is_empty() {
            return Err(invalid("python-environment-identity-invalid"));
        }
        let wheels = self
            .root
            .join("environments")
            .join(environment_sha256)
            .join("wheels");
        let package = package.replace('_', "-").to_ascii_lowercase();
        fs::read_dir(wheels)?
            .filter_map(Result::ok)
            .map(|entry| entry.path())
            .find(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .and_then(|name| name.split('-').next())
                    .map(|name| name.replace('_', "-").to_ascii_lowercase() == package)
                    .unwrap_or(false)
            })
            .ok_or_else(|| invalid("python-environment-wheel-not-ready"))
    }

    pub fn find_by_lock_file_sha256(
        &self,
        lock_file_sha256: &str,
    ) -> Result<Option<EnvironmentRecord>, PythonResearchError> {
        let _gate = self
            .gate
            .lock()
            .map_err(|_| invalid("python-environment-store-lock-poisoned"))?;
        if !is_sha256(lock_file_sha256) {
            return Err(invalid("python-environment-lock-file-identity-invalid"));
        }
        let directory = self.root.join("environments");
        if !directory.is_dir() {
            return Ok(None);
        }
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            if !entry.file_type()?.is_dir() {
                continue;
            }
            let destination = entry.path();
            let lock_bytes = match fs::read(destination.join("pylock.toml")) {
                Ok(bytes) if sha256(&bytes) == lock_file_sha256 => bytes,
                _ => continue,
            };
            let lock = match std::str::from_utf8(&lock_bytes)
                .ok()
                .and_then(|text| toml::from_str::<EnvironmentLock>(text).ok())
            {
                Some(lock) => lock,
                None => continue,
            };
            lock.validate()?;
            for wheel in &lock.wheels {
                let payload = fs::read(destination.join("wheels").join(&wheel.file_name))?;
                wheel.validate(&payload, lock.platform)?;
            }
            let environment_sha256 =
                sha256(&serde_json::to_vec(&lock).map_err(|error| invalid(error.to_string()))?);
            if entry.file_name().to_string_lossy() != environment_sha256 {
                continue;
            }
            return Ok(Some(EnvironmentRecord {
                environment_sha256,
                path: destination.to_string_lossy().into_owned(),
                lock_sha256: lock.lock_sha256,
                wheel_count: lock.wheels.len(),
            }));
        }
        Ok(None)
    }

    pub fn load_lock(
        &self,
        environment_sha256: &str,
    ) -> Result<EnvironmentLock, PythonResearchError> {
        if !is_sha256(environment_sha256) {
            return Err(invalid("python-environment-identity-invalid"));
        }
        let bytes = fs::read(
            self.root
                .join("environments")
                .join(environment_sha256)
                .join("pylock.toml"),
        )?;
        let lock: EnvironmentLock = toml::from_str(
            std::str::from_utf8(&bytes)
                .map_err(|_| invalid("python-environment-lock-unreadable:utf8"))?,
        )
        .map_err(|error| invalid(format!("python-environment-lock-unreadable:{error}")))?;
        lock.validate()?;
        Ok(lock)
    }
}

fn validate_runtime_archive(payload: &[u8]) -> Result<(), PythonResearchError> {
    if payload.starts_with(&[0x1f, 0x8b]) {
        return validate_tar_runtime_archive(payload);
    }
    validate_zip_runtime_archive(payload)
}

fn validate_zip_runtime_archive(payload: &[u8]) -> Result<(), PythonResearchError> {
    let mut archive = ZipArchive::new(Cursor::new(payload))
        .map_err(|_| invalid("python-runtime-archive-invalid"))?;
    let mut found_manifest = false;
    let mut found_launcher = false;
    let mut paths = BTreeSet::new();
    let mut folded_paths = BTreeSet::new();
    let mut expanded_bytes = 0u64;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
        let name = file.name().to_owned();
        if name.is_empty()
            || name.contains("..")
            || name.contains('\\')
            || name.starts_with('/')
            || file.is_dir()
            || !paths.insert(name.clone())
            || !folded_paths.insert(name.to_ascii_lowercase())
        {
            return Err(invalid("python-runtime-archive-path-invalid"));
        }
        if file
            .unix_mode()
            .is_some_and(|mode| mode & 0o170000 == 0o120000)
        {
            return Err(invalid("python-runtime-archive-symbolic-link-not-allowed"));
        }
        if file.unix_mode().is_some_and(|mode| {
            let file_type = mode & 0o170000;
            file_type != 0 && file_type != 0o100000
        }) {
            return Err(invalid("python-runtime-archive-special-file-not-allowed"));
        }
        if name == "runtime/manifest.json" {
            found_manifest = true;
        }
        if name == "runtime/python" || name == "runtime/python.exe" {
            found_launcher = true;
        }
        if file.size() > MAX_RUNTIME_ARCHIVE_ENTRY_BYTES {
            return Err(invalid("python-runtime-archive-entry-too-large"));
        }
        expanded_bytes = expanded_bytes
            .checked_add(file.size())
            .ok_or_else(|| invalid("python-runtime-archive-expanded-size-invalid"))?;
        if expanded_bytes > MAX_RUNTIME_ARCHIVE_EXPANDED_BYTES {
            return Err(invalid("python-runtime-archive-expanded-size-exceeded"));
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| invalid("python-runtime-archive-entry-unreadable"))?;
        if name == "runtime/manifest.json"
            && !serde_json::from_slice::<serde_json::Value>(&bytes)
                .ok()
                .is_some_and(|value| value.is_object())
        {
            return Err(invalid("python-runtime-archive-manifest-invalid"));
        }
    }
    if !found_manifest || !found_launcher {
        return Err(invalid("python-runtime-archive-structure-invalid"));
    }
    Ok(())
}

fn runtime_archive_installed_bytes(payload: &[u8]) -> Result<u64, PythonResearchError> {
    if payload.starts_with(&[0x1f, 0x8b]) {
        let decoder = GzDecoder::new(Cursor::new(payload));
        let mut archive = Archive::new(decoder);
        let mut total = 0u64;
        for entry in archive
            .entries()
            .map_err(|_| invalid("python-runtime-archive-invalid"))?
        {
            let entry = entry.map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
            if entry.header().entry_type().is_file() {
                total = total
                    .checked_add(entry.size())
                    .ok_or_else(|| invalid("python-runtime-archive-expanded-size-invalid"))?;
            }
        }
        return Ok(total);
    }
    let mut archive = ZipArchive::new(Cursor::new(payload))
        .map_err(|_| invalid("python-runtime-archive-invalid"))?;
    let mut total = 0u64;
    for index in 0..archive.len() {
        let file = archive
            .by_index(index)
            .map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
        if !file.is_dir() {
            total = total
                .checked_add(file.size())
                .ok_or_else(|| invalid("python-runtime-archive-expanded-size-invalid"))?;
        }
    }
    Ok(total)
}

fn validate_tar_runtime_archive(payload: &[u8]) -> Result<(), PythonResearchError> {
    let decoder = GzDecoder::new(Cursor::new(payload));
    let mut archive = Archive::new(decoder);
    let mut paths = BTreeSet::new();
    let mut folded_paths = BTreeSet::new();
    let mut links = Vec::new();
    let mut found_launcher = false;
    let mut expanded_bytes = 0u64;
    for entry in archive
        .entries()
        .map_err(|_| invalid("python-runtime-archive-invalid"))?
    {
        let mut entry = entry.map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
        let entry_path = entry
            .path()
            .map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
        let name = archive_path(entry_path.as_ref())?;
        if !paths.insert(name.clone()) || !folded_paths.insert(name.to_ascii_lowercase()) {
            return Err(invalid("python-runtime-archive-path-invalid"));
        }
        let entry_type = entry.header().entry_type();
        if entry_type.is_file() {
            let size = entry.size();
            if size > MAX_RUNTIME_ARCHIVE_ENTRY_BYTES {
                return Err(invalid("python-runtime-archive-entry-too-large"));
            }
            expanded_bytes = expanded_bytes
                .checked_add(size)
                .ok_or_else(|| invalid("python-runtime-archive-expanded-size-invalid"))?;
            if expanded_bytes > MAX_RUNTIME_ARCHIVE_EXPANDED_BYTES {
                return Err(invalid("python-runtime-archive-expanded-size-exceeded"));
            }
            io::copy(&mut entry, &mut io::sink())
                .map_err(|_| invalid("python-runtime-archive-entry-unreadable"))?;
            found_launcher |= name == "python/bin/python3.12" || name == "python/python.exe";
        } else if entry_type.is_dir() {
            if name == "python/bin" || name == "python" {
                found_launcher |= false;
            }
        } else if entry_type.is_symlink() {
            let target = entry
                .link_name()
                .map_err(|_| invalid("python-runtime-archive-link-invalid"))?
                .ok_or_else(|| invalid("python-runtime-archive-link-invalid"))?;
            let target = target
                .to_str()
                .ok_or_else(|| invalid("python-runtime-archive-link-invalid"))?;
            links.push((name, target.to_owned()));
        } else {
            return Err(invalid("python-runtime-archive-special-file-not-allowed"));
        }
    }
    for (name, target) in links {
        let resolved = resolve_archive_link(&name, &target)?;
        if !paths.contains(&resolved) {
            return Err(invalid("python-runtime-archive-link-invalid"));
        }
    }
    if !found_launcher {
        return Err(invalid("python-runtime-archive-structure-invalid"));
    }
    Ok(())
}

fn archive_path(path: &Path) -> Result<String, PythonResearchError> {
    let value = path
        .to_str()
        .ok_or_else(|| invalid("python-runtime-archive-path-invalid"))?;
    if value.is_empty() || value.contains('\\') || value.starts_with('/') {
        return Err(invalid("python-runtime-archive-path-invalid"));
    }
    let mut components = Vec::new();
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return Err(invalid("python-runtime-archive-path-invalid"));
        }
        components.push(component);
    }
    Ok(components.join("/"))
}

fn resolve_archive_link(name: &str, target: &str) -> Result<String, PythonResearchError> {
    if target.is_empty() || target.contains('\\') || target.starts_with('/') {
        return Err(invalid("python-runtime-archive-link-invalid"));
    }
    let mut components = name.split('/').collect::<Vec<_>>();
    components.pop();
    for component in target.split('/') {
        match component {
            "" | "." => {}
            ".." => {
                components
                    .pop()
                    .ok_or_else(|| invalid("python-runtime-archive-link-invalid"))?;
            }
            value => components.push(value),
        }
    }
    if components.iter().any(|component| *component == "..") {
        return Err(invalid("python-runtime-archive-link-invalid"));
    }
    Ok(components.join("/"))
}

fn extract_runtime_archive(payload: &[u8], staging: &Path) -> Result<(), PythonResearchError> {
    if payload.starts_with(&[0x1f, 0x8b]) {
        let decoder = GzDecoder::new(Cursor::new(payload));
        let mut archive = Archive::new(decoder);
        let mut links = Vec::new();
        for entry in archive
            .entries()
            .map_err(|_| invalid("python-runtime-archive-invalid"))?
        {
            let mut entry = entry.map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
            if entry.header().entry_type().is_symlink() {
                let entry_path = entry
                    .path()
                    .map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
                let name = archive_path(entry_path.as_ref())?;
                let target = entry
                    .link_name()
                    .map_err(|_| invalid("python-runtime-archive-link-invalid"))?
                    .ok_or_else(|| invalid("python-runtime-archive-link-invalid"))?;
                let target = target
                    .to_str()
                    .ok_or_else(|| invalid("python-runtime-archive-link-invalid"))?;
                links.push((name, target.to_owned()));
                continue;
            }
            if !entry.header().entry_type().is_file() {
                continue;
            }
            let entry_path = entry
                .path()
                .map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
            let name = archive_path(entry_path.as_ref())?;
            let path = staging.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            let mut output = fs::File::create(&path)?;
            io::copy(&mut entry, &mut output)
                .map_err(|_| invalid("python-runtime-archive-entry-unpack-failed"))?;
            #[cfg(unix)]
            if let Ok(mode) = entry.header().mode() {
                fs::set_permissions(&path, fs::Permissions::from_mode(mode & 0o777))?;
            }
        }
        for (name, target) in links {
            let path = staging.join(name);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent)?;
            }
            #[cfg(unix)]
            std::os::unix::fs::symlink(target, path)?;
            #[cfg(not(unix))]
            {
                let _ = (target, path);
                return Err(invalid("python-runtime-archive-link-unsupported"));
            }
        }
        return Ok(());
    }
    let mut archive = ZipArchive::new(Cursor::new(payload))
        .map_err(|_| invalid("python-runtime-archive-invalid"))?;
    for index in 0..archive.len() {
        let mut file = archive
            .by_index(index)
            .map_err(|_| invalid("python-runtime-archive-entry-invalid"))?;
        if file.is_dir() {
            continue;
        }
        let path = staging.join(file.name());
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)
            .map_err(|_| invalid("python-runtime-archive-entry-unreadable"))?;
        fs::write(&path, bytes)?;
        #[cfg(unix)]
        if let Some(mode) = file.unix_mode() {
            fs::set_permissions(path, fs::Permissions::from_mode(mode & 0o777))?;
        }
    }
    Ok(())
}

fn runtime_executable_exists(directory: &Path) -> bool {
    [
        directory.join("runtime/python"),
        directory.join("runtime/python.exe"),
        directory.join("python/bin/python3.12"),
        directory.join("python/python.exe"),
    ]
    .into_iter()
    .any(|path| {
        path.is_file() && {
            #[cfg(unix)]
            {
                fs::metadata(&path)
                    .map(|metadata| metadata.permissions().mode() & 0o111 != 0)
                    .unwrap_or(false)
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
    })
}

fn runtime_cache_matches(directory: &Path, manifest: &RuntimeArtifactManifest) -> bool {
    if !directory.is_dir() || !runtime_executable_exists(directory) {
        return false;
    }
    fs::read(directory.join("adaq-runtime-manifest.json"))
        .ok()
        .and_then(|bytes| serde_json::from_slice::<RuntimeArtifactManifest>(&bytes).ok())
        .is_some_and(|stored| stored == *manifest)
}

fn staging_path(destination: &Path) -> Result<PathBuf, PythonResearchError> {
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("python-cache-path-invalid"))?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("python-cache-name-invalid"))?;
    Ok(parent.join(format!(".{name}.staging-{}", unique_suffix())))
}

fn replace_directory(staging: &Path, destination: &Path) -> Result<(), PythonResearchError> {
    let parent = destination
        .parent()
        .ok_or_else(|| invalid("python-cache-path-invalid"))?;
    fs::create_dir_all(parent)?;
    let name = destination
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| invalid("python-cache-name-invalid"))?;
    let backup = parent.join(format!(".{name}.backup-{}", unique_suffix()));
    let had_destination = destination.exists();
    if had_destination {
        fs::rename(destination, &backup)?;
    }
    match fs::rename(staging, destination) {
        Ok(()) => {
            if had_destination {
                let _ = remove_cache_path(&backup);
            }
            Ok(())
        }
        Err(error) => {
            if had_destination {
                let _ = fs::rename(&backup, destination);
            }
            Err(error.into())
        }
    }
}

fn remove_cache_path(path: &Path) -> io::Result<()> {
    if path.is_dir() {
        fs::remove_dir_all(path)
    } else {
        fs::remove_file(path)
    }
}

fn directory_bytes(path: &Path) -> u64 {
    if path.is_file() {
        return fs::metadata(path)
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }
    fs::read_dir(path)
        .ok()
        .into_iter()
        .flatten()
        .filter_map(|entry| entry.ok())
        .filter(|entry| !entry.file_type().is_ok_and(|kind| kind.is_symlink()))
        .map(|entry| directory_bytes(&entry.path()))
        .fold(0, u64::saturating_add)
}

fn persist_json<T: Serialize>(path: &Path, value: &T) -> Result<(), PythonResearchError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let temporary = path.with_extension("json.tmp");
    fs::write(
        &temporary,
        serde_json::to_vec_pretty(value).map_err(|error| invalid(error.to_string()))?,
    )?;
    replace_file(&temporary, path)?;
    Ok(())
}

fn replace_file(temporary: &Path, destination: &Path) -> Result<(), PythonResearchError> {
    let backup = destination.with_extension("bak");
    if backup.exists() {
        fs::remove_file(&backup)?;
    }
    let had_destination = destination.exists();
    if had_destination {
        fs::rename(destination, &backup)?;
    }
    match fs::rename(temporary, destination) {
        Ok(()) => {
            if had_destination {
                let _ = fs::remove_file(backup);
            }
            Ok(())
        }
        Err(error) => {
            if had_destination {
                let _ = fs::rename(backup, destination);
            }
            Err(error.into())
        }
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default()
}

fn hex_bytes(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::tempdir;
    use zip::{ZipWriter, write::SimpleFileOptions};

    fn runtime_archive() -> Vec<u8> {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("runtime/manifest.json", b"{}".as_slice()),
            ("runtime/python", b"managed-cpython".as_slice()),
        ] {
            writer
                .start_file(
                    name,
                    SimpleFileOptions::default().unix_permissions(if name.ends_with("python") {
                        0o755
                    } else {
                        0o644
                    }),
                )
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    fn runtime_manifest(payload: &[u8]) -> RuntimeArtifactManifest {
        let mut manifest = RuntimeArtifactManifest {
            schema: RUNTIME_MANIFEST_SCHEMA.into(),
            profile: PYTHON_RUNTIME_PROFILE.into(),
            version: "3.12.9".into(),
            platform: RuntimePlatform::current().unwrap(),
            source: "https://example.invalid/adaq-cpython.zip".into(),
            license: "Python-2.0".into(),
            file_name: "adaq-cpython.zip".into(),
            download_bytes: payload.len() as u64,
            installed_bytes: runtime_archive_installed_bytes(payload).unwrap(),
            artifact_sha256: sha256(payload),
            signature: String::new(),
        };
        manifest.signature = manifest.expected_signature().unwrap();
        manifest
    }

    fn tar_runtime_archive() -> Vec<u8> {
        let encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        let mut builder = tar::Builder::new(encoder);
        let bytes = b"managed-cpython";
        let mut header = tar::Header::new_gnu();
        header.set_path("python/bin/python3.12").unwrap();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        builder.append(&header, bytes.as_slice()).unwrap();
        builder.into_inner().unwrap().finish().unwrap()
    }

    fn wheel(name: &str, package: &str) -> (WheelIdentity, Vec<u8>) {
        let payload = format!("wheel:{name}").into_bytes();
        (
            WheelIdentity {
                file_name: name.into(),
                package: package.into(),
                version: "1.0.0".into(),
                sha256: sha256(&payload),
                size: payload.len() as u64,
                platform_tags: vec!["any".into()],
            },
            payload,
        )
    }

    fn wheelhouse() -> (WheelhouseManifest, BTreeMap<String, Vec<u8>>) {
        let mut wheels = Vec::new();
        let mut payloads = BTreeMap::new();
        for (package, file) in [
            (
                "adaq-research-sdk",
                "adaq_research_sdk-1.0.0-py3-none-any.whl",
            ),
            (
                "adaq-python-research-runner",
                "adaq_python_research_runner-1.0.0-py3-none-any.whl",
            ),
            (
                "adaq-qlib-ridge-adapter",
                "adaq_qlib_ridge_adapter-1.0.0-py3-none-any.whl",
            ),
            ("pyarrow", "pyarrow-1.0.0-py3-none-any.whl"),
            ("numpy", "numpy-1.0.0-py3-none-any.whl"),
        ] {
            let (identity, payload) = wheel(file, package);
            payloads.insert(identity.file_name.clone(), payload);
            wheels.push(identity);
        }
        let mut manifest = WheelhouseManifest {
            schema: WHEELHOUSE_MANIFEST_SCHEMA.into(),
            identity: String::new(),
            runtime_profile: PYTHON_RUNTIME_PROFILE.into(),
            platform: RuntimePlatform::current().unwrap(),
            signature: String::new(),
            wheels,
        };
        manifest.identity = wheelhouse_identity(&manifest.wheels).unwrap();
        manifest.signature = manifest.expected_signature().unwrap();
        (manifest, payloads)
    }

    #[test]
    fn runtime_preparation_verifies_signature_structure_and_atomic_cache() {
        let payload = runtime_archive();
        let manifest = runtime_manifest(&payload);
        manifest.validate(&payload).unwrap();
        let directory = tempdir().unwrap();
        let store = RuntimeStore::new(directory.path());
        let (attempt, record) = store
            .prepare("alice", &manifest, &payload, || false)
            .unwrap();
        assert_eq!(attempt.status, PreparationStatus::Ready);
        let record = record.unwrap();
        assert!(record.path.ends_with(&manifest.artifact_sha256));
        assert!(store.executable_path(&manifest.artifact_sha256).is_ok());
        store.disable_execution(&manifest.artifact_sha256).unwrap();
        assert_eq!(
            store
                .executable_path(&manifest.artifact_sha256)
                .unwrap_err()
                .0,
            "python-runtime-security-disabled"
        );
        let mut invalid = manifest.clone();
        invalid.signature = "bad".into();
        assert!(invalid.validate(&payload).is_err());
        let (cancelled, record) = store
            .prepare("alice", &manifest, &payload, || true)
            .unwrap();
        assert_eq!(cancelled.status, PreparationStatus::Cancelled);
        assert!(record.is_none());
        assert!(
            store
                .evict_inactive(&BTreeSet::new())
                .unwrap()
                .contains(&manifest.artifact_sha256)
        );
    }

    #[test]
    fn runtime_preparation_retries_are_retained_and_failed_staging_keeps_cache() {
        let payload = runtime_archive();
        let manifest = runtime_manifest(&payload);
        let directory = tempdir().unwrap();
        let store = RuntimeStore::new(directory.path());
        let (first, record) = store
            .prepare("alice", &manifest, &payload, || false)
            .unwrap();
        assert_eq!(first.status, PreparationStatus::Ready);
        assert!(record.is_some());

        let (retry, record) = store
            .prepare_with_source(
                "alice",
                Some(first.attempt_id.clone()),
                &manifest,
                &payload,
                || false,
            )
            .unwrap();
        assert_ne!(retry.attempt_id, first.attempt_id);
        assert_eq!(
            retry.source_attempt_id.as_deref(),
            Some(first.attempt_id.as_str())
        );
        assert!(record.is_some());
        assert!(
            store
                .prepare_with_source(
                    "bob",
                    Some(first.attempt_id.clone()),
                    &manifest,
                    &payload,
                    || false,
                )
                .is_err()
        );

        let invalid_payload = b"corrupt-runtime";
        assert!(
            store
                .prepare("alice", &manifest, invalid_payload, || false)
                .is_err()
        );
        assert!(store.executable_path(&manifest.artifact_sha256).is_ok());
        let attempts = store.list_preparations("alice").unwrap();
        assert_eq!(attempts.len(), 3);
        assert_eq!(attempts.last().unwrap().status, PreparationStatus::Failed);
    }

    #[test]
    fn interrupted_runtime_preparations_recover_as_retryable_failures() {
        let directory = tempdir().unwrap();
        let attempt = PreparationAttempt {
            attempt_id: "runtime-1".into(),
            user_id: "alice".into(),
            status: PreparationStatus::Preparing,
            identity: None,
            source_attempt_id: None,
            diagnostic: None,
        };
        persist_json(
            &directory.path().join("runtime-preparations.json"),
            &PreparationDatabase {
                attempts: BTreeMap::from([(attempt.attempt_id.clone(), attempt)]),
            },
        )
        .unwrap();
        let store = RuntimeStore::new(directory.path());
        let recovered = store.list_preparations("alice").unwrap();
        assert_eq!(recovered[0].status, PreparationStatus::Failed);
        assert_eq!(
            recovered[0].diagnostic.as_deref(),
            Some("python-runtime-preparation-interrupted")
        );
    }

    #[test]
    fn wheelhouse_cache_is_verified_separate_and_evictable() {
        let (manifest, payloads) = wheelhouse();
        let directory = tempdir().unwrap();
        let store = WheelhouseStore::new(directory.path());
        let record = store.prepare(&manifest, &payloads).unwrap();
        assert_eq!(record.identity, manifest.identity);
        assert!(record.disk_bytes > 0);
        let (loaded_manifest, loaded_payloads) = store.load(&manifest.identity).unwrap();
        assert_eq!(loaded_manifest, manifest);
        assert_eq!(loaded_payloads, payloads);
        assert_eq!(
            store.evict_inactive(&BTreeSet::new()).unwrap(),
            vec![manifest.identity.clone()]
        );
        assert!(store.load(&manifest.identity).is_err());
    }

    #[test]
    fn tar_runtime_preparation_accepts_pinned_python_standalone_layout() {
        let payload = tar_runtime_archive();
        let manifest = runtime_manifest(&payload);
        manifest.validate(&payload).unwrap();
        let directory = tempdir().unwrap();
        let store = RuntimeStore::new(directory.path());
        let (_, record) = store
            .prepare("alice", &manifest, &payload, || false)
            .unwrap();
        assert!(record.is_some());
        assert!(store.executable_path(&manifest.artifact_sha256).is_ok());
    }

    #[test]
    fn runtime_catalog_is_complete_for_supported_platforms() {
        let entries = runtime_catalog();
        assert_eq!(entries.len(), 3);
        for entry in entries {
            assert_eq!(
                entry.manifest.expected_signature().unwrap(),
                entry.manifest.signature
            );
            assert!(entry.download_url.contains("20260807"));
            assert!(entry.manifest.download_bytes > 0);
            assert!(entry.manifest.installed_bytes > entry.manifest.download_bytes);
        }
    }

    #[test]
    fn wheelhouse_catalog_is_signed_and_uses_embedded_control_wheels() {
        for platform in [
            RuntimePlatform::MacosAarch64,
            RuntimePlatform::WindowsX86_64,
            RuntimePlatform::LinuxX86_64,
        ] {
            let catalog = wheelhouse_catalog(platform).unwrap();
            assert_eq!(
                catalog.manifest.expected_signature().unwrap(),
                catalog.manifest.signature
            );
            assert_eq!(
                catalog.manifest.identity,
                wheelhouse_identity(&catalog.manifest.wheels).unwrap()
            );
            for package in REQUIRED_WHEEL_PACKAGES {
                assert!(
                    catalog
                        .manifest
                        .wheels
                        .iter()
                        .any(|wheel| wheel.package == package)
                );
            }
            assert_eq!(catalog.download_urls.len(), 2);
            for wheel in &catalog.manifest.wheels {
                let payload = embedded_wheel_payload(&wheel.file_name);
                if let Some(payload) = payload {
                    wheel.validate(payload, platform).unwrap();
                } else {
                    assert!(catalog.download_urls.contains_key(&wheel.file_name));
                }
            }
        }
    }

    #[test]
    fn runtime_archive_rejects_ambiguous_entries_and_non_object_manifests() {
        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("runtime/manifest.json", b"{}".as_slice()),
            ("runtime/python", b"managed-cpython".as_slice()),
            ("RUNTIME/PYTHON", b"duplicate".as_slice()),
        ] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        let duplicate = writer.finish().unwrap().into_inner();
        assert_eq!(
            validate_runtime_archive(&duplicate).unwrap_err().0,
            "python-runtime-archive-path-invalid"
        );

        let mut writer = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("runtime/manifest.json", b"[]".as_slice()),
            ("runtime/python", b"managed-cpython".as_slice()),
        ] {
            writer
                .start_file(name, SimpleFileOptions::default())
                .unwrap();
            writer.write_all(bytes).unwrap();
        }
        let invalid_manifest = writer.finish().unwrap().into_inner();
        assert_eq!(
            validate_runtime_archive(&invalid_manifest).unwrap_err().0,
            "python-runtime-archive-manifest-invalid"
        );
    }

    #[test]
    fn wheel_only_sync_is_deterministic_and_rejects_source_distributions() {
        let (manifest, payloads) = wheelhouse();
        manifest.validate(&payloads).unwrap();
        let intent = DependencyIntent::parse(
            br#"[project]
name = "tutorial"
dependencies = ["numpy==1.0.0"]
"#,
        )
        .unwrap();
        let runtime = sha256(b"runtime");
        let lock = sync_environment(
            runtime.as_str(),
            manifest.platform,
            &intent,
            &manifest,
            &payloads,
        )
        .unwrap();
        assert!(lock.validate().is_ok());
        let directory = tempdir().unwrap();
        let store = EnvironmentStore::new(directory.path());
        let record = store.prepare(&lock, &payloads, &manifest).unwrap();
        let lock_bytes = std::fs::read(
            directory
                .path()
                .join("environments")
                .join(&record.environment_sha256)
                .join("pylock.toml"),
        )
        .unwrap();
        assert!(
            std::str::from_utf8(&lock_bytes)
                .unwrap()
                .contains("schema = \"adaq-environment-lock@1\"")
        );
        let lock_file_sha256 = sha256(&lock_bytes);
        assert_eq!(
            store
                .find_by_lock_file_sha256(&lock_file_sha256)
                .unwrap()
                .unwrap()
                .environment_sha256,
            record.environment_sha256
        );
        assert_eq!(store.load_lock(&record.environment_sha256).unwrap(), lock);
        let wheel_path = directory
            .path()
            .join("environments")
            .join(&record.environment_sha256)
            .join("wheels")
            .join("numpy-1.0.0-py3-none-any.whl");
        std::fs::write(&wheel_path, b"corrupt").unwrap();
        store.prepare(&lock, &payloads, &manifest).unwrap();
        assert_eq!(
            std::fs::read(wheel_path).unwrap(),
            payloads["numpy-1.0.0-py3-none-any.whl"]
        );
        assert!(
            DependencyIntent::parse(
                br#"[project]
name = "bad"
dependencies = ["evil @ https://example.invalid/evil.tar.gz"]
"#,
            )
            .is_err()
        );
    }
}
