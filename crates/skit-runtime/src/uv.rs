//! Install a verified private uv binary when the host does not provide uv.

use std::{
    fs::{self, File, OpenOptions},
    io::{self, Cursor, Read as _, Write as _},
    path::{Path, PathBuf},
    time::Duration,
};

use flate2::read::GzDecoder;
use sha2::{Digest as _, Sha256};
use skit_i18n::{Localize, Message};
use thiserror::Error;
use uuid::Uuid;

/// The uv version that the private bootstrap installs.
pub const UV_VERSION: &str = "0.12.3";

const OFFICIAL_BASE: &str = "https://github.com/astral-sh/uv/releases/download";
const MAX_ARCHIVE_BYTES: u64 = 100 * 1024 * 1024;

const CHECKSUMS: &[(&str, &str)] = &[
    (
        "aarch64-apple-darwin",
        "546f7f8a6c70ff13a3a9d2bc958db3427298cebf3e0cb756f9177133b7068843",
    ),
    (
        "x86_64-apple-darwin",
        "4c9f52262a14da336e4a42ed24992d12d0c956acde87619e4611d321dffa602b",
    ),
    (
        "aarch64-unknown-linux-gnu",
        "bb66cb52e7b1823aed1183630d8d8e5c958840d584a4c55ec10a4cfc168dcca2",
    ),
    (
        "x86_64-unknown-linux-gnu",
        "600cf9a742aca00d292673b16b5acffaa7b8c269a364ad0c2e79498dcb1fe101",
    ),
    (
        "aarch64-unknown-linux-musl",
        "fa513fca1eb2913334c944fe9adbdd410274a1cbe8dd05d03699a9eb85311d4e",
    ),
    (
        "x86_64-unknown-linux-musl",
        "0643b9fb8c9fb27458e709ce6ff939695013c41975ff7b02d3f3b138d8d4bdb3",
    ),
    (
        "aarch64-pc-windows-msvc",
        "4343217d668727b8a8eb5cad92389a1d2eeead93c89940d1b955ba1bb15462eb",
    ),
    (
        "x86_64-pc-windows-msvc",
        "b23350c79e8ad0192b8124af13a0f17e8d4e4549524785e1aef389ae5a06990e",
    ),
];

/// Identify one supported uv release target.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UvTarget {
    triple: String,
    windows: bool,
}

impl UvTarget {
    /// Resolve a target from normalized architecture and operating-system names.
    pub fn from_parts(arch: &str, os: &str, musl: bool) -> Result<Self, UvBootstrapError> {
        let arch = match arch.to_ascii_lowercase().as_str() {
            "x86_64" | "amd64" => "x86_64",
            "aarch64" | "arm64" => "aarch64",
            _ => {
                return Err(UvBootstrapError::UnsupportedPlatform {
                    platform: format!("{os}/{arch}"),
                });
            }
        };
        let (suffix, windows) = match os {
            "macos" | "darwin" => ("apple-darwin", false),
            "windows" => ("pc-windows-msvc", true),
            "linux" if musl => ("unknown-linux-musl", false),
            "linux" => ("unknown-linux-gnu", false),
            _ => {
                return Err(UvBootstrapError::UnsupportedPlatform {
                    platform: format!("{os}/{arch}"),
                });
            }
        };
        Ok(Self {
            triple: format!("{arch}-{suffix}"),
            windows,
        })
    }

    /// Resolve the current build target.
    pub fn current() -> Result<Self, UvBootstrapError> {
        Self::from_parts(
            std::env::consts::ARCH,
            std::env::consts::OS,
            host_uses_musl(),
        )
    }

    /// Return the release target triple.
    #[must_use]
    pub fn triple(&self) -> &str {
        &self.triple
    }
}

/// Describe one authenticated uv release archive.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UvAsset {
    /// Release version.
    pub version: String,
    /// Archive filename.
    pub filename: String,
    /// Download URL.
    pub url: String,
    /// Expected SHA-256 digest.
    pub checksum: String,
    /// Executable filename inside the archive and installation directory.
    pub executable_name: String,
}

/// Ask before skit downloads its private uv.
///
/// Version 0.4 asks once, on the first Python run that finds no uv (`src/skit/uvman.py:63-88`).
/// Pulling an executable from the network is not something skit does silently.
pub trait UvDownloadConsent: std::fmt::Debug {
    /// Return true when the download can start.
    ///
    /// `destination` is the private directory that receives the executable.
    fn allow_download(&self, version: &str, destination: &Path) -> bool;
}

/// Consent that never asks.
///
/// Version 0.4 keeps its zero-action first run whenever there is nobody to ask
/// (`src/skit/uvman.py:72-73`).
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct AllowUvDownload;

impl UvDownloadConsent for AllowUvDownload {
    fn allow_download(&self, _version: &str, _destination: &Path) -> bool {
        true
    }
}

/// Report a private uv bootstrap failure.
#[derive(Debug, Error)]
pub enum UvBootstrapError {
    /// The user answered no to the download question.
    #[error(
        "Download declined. Install uv yourself (https://docs.astral.sh/uv/getting-started/installation/) and skit will pick it up automatically."
    )]
    Declined,
    /// This build target does not have a pinned release archive.
    #[error("unsupported platform: {platform}")]
    UnsupportedPlatform { platform: String },
    /// The configured mirror or official server could not provide the archive.
    #[error("could not download uv from {url}: {reason}")]
    Download { url: String, reason: String },
    /// The archive digest did not match the official digest.
    #[error(
        "Downloaded uv failed its checksum (the mirror may be compromised or the file corrupt). Expected {expected}, got {actual}."
    )]
    Checksum {
        /// Pinned official digest.
        expected: String,
        /// Digest of the downloaded bytes.
        actual: String,
    },
    /// The build target has no pinned digest to verify against.
    #[error("No pinned checksum for platform {triple}; refusing to run an unverified uv.")]
    NoPinnedChecksum {
        /// Target triple with no pin.
        triple: String,
    },
    /// The authenticated archive did not contain the expected executable.
    #[error("the uv archive is invalid: {reason}")]
    Archive { reason: String },
    /// A private installation file operation failed.
    #[error("could not {operation} private uv at {path}: {source}")]
    Io {
        /// Operation such as create, lock, write, or rename.
        operation: &'static str,
        /// Affected path.
        path: String,
        /// Operating-system detail.
        #[source]
        source: io::Error,
    },
}

impl Localize for UvBootstrapError {
    fn message(&self) -> Message {
        match self {
            Self::Declined => Message::new(
                "Download declined. Install uv yourself (https://docs.astral.sh/uv/getting-started/installation/) and skit will pick it up automatically.",
            ),
            Self::UnsupportedPlatform { platform } => {
                Message::new("unsupported platform: {}").with(platform)
            }
            Self::Download { url, reason } => Message::new("could not download uv from {}: {}")
                .with(url)
                .with(reason),
            Self::Checksum { expected, actual } => Message::new(
                "Downloaded uv failed its checksum (the mirror may be compromised or the file corrupt). Expected {}, got {}.",
            )
            .with(expected)
            .with(actual),
            Self::NoPinnedChecksum { triple } => {
                Message::new("No pinned checksum for platform {}; refusing to run an unverified uv.")
                    .with(triple)
            }
            Self::Archive { reason } => Message::new("the uv archive is invalid: {}").with(reason),
            Self::Io {
                operation,
                path,
                source,
            } => Message::new("could not {} private uv at {}: {}")
                .nested(Message::term(operation))
                .with(path)
                .with(source),
        }
    }
}

/// Build the current authenticated asset description for one target.
///
/// Fails closed: a triple with no pinned digest refuses rather than describing an
/// archive skit could never authenticate.
pub fn uv_asset(target: &UvTarget, mirror_base: Option<&str>) -> Result<UvAsset, UvBootstrapError> {
    let extension = if target.windows { "zip" } else { "tar.gz" };
    let filename = format!("uv-{}.{extension}", target.triple);
    let base = mirror_base.unwrap_or(OFFICIAL_BASE).trim_end_matches('/');
    let checksum = pinned_checksum(&target.triple)?;
    Ok(UvAsset {
        version: UV_VERSION.to_owned(),
        url: format!("{base}/{UV_VERSION}/{filename}"),
        filename,
        checksum: checksum.to_owned(),
        executable_name: if target.windows { "uv.exe" } else { "uv" }.to_owned(),
    })
}

fn pinned_checksum(triple: &str) -> Result<&'static str, UvBootstrapError> {
    CHECKSUMS
        .iter()
        .find_map(|(pinned, checksum)| (*pinned == triple).then_some(*checksum))
        .ok_or_else(|| UvBootstrapError::NoPinnedChecksum {
            triple: triple.to_owned(),
        })
}

/// Return the private uv path when it is already installed.
#[must_use]
pub fn managed_uv_path(data_dir: &Path) -> PathBuf {
    data_dir
        .join("bin")
        .join(if cfg!(windows) { "uv.exe" } else { "uv" })
}

/// Download and install uv below the skit data directory.
pub fn ensure_managed_uv(
    data_dir: &Path,
    mirror_base: Option<&str>,
) -> Result<PathBuf, UvBootstrapError> {
    let destination = managed_uv_path(data_dir);
    if destination.is_file() {
        return Ok(destination);
    }
    let asset = uv_asset(&UvTarget::current()?, mirror_base)?;
    ensure_managed_uv_from_asset(data_dir, &asset, download_archive)
}

fn ensure_managed_uv_from_asset<F>(
    data_dir: &Path,
    asset: &UvAsset,
    fetch: F,
) -> Result<PathBuf, UvBootstrapError>
where
    F: FnOnce(&UvAsset) -> Result<Vec<u8>, UvBootstrapError>,
{
    let mut operations = SystemUvInstallOperations;
    ensure_managed_uv_from_asset_with_operations(data_dir, asset, fetch, &mut operations)
}

fn ensure_managed_uv_from_asset_with_operations<F>(
    data_dir: &Path,
    asset: &UvAsset,
    fetch: F,
    operations: &mut impl UvInstallOperations,
) -> Result<PathBuf, UvBootstrapError>
where
    F: FnOnce(&UvAsset) -> Result<Vec<u8>, UvBootstrapError>,
{
    let destination = managed_uv_path(data_dir);
    let bin = destination
        .parent()
        .expect("a managed executable always has a parent");
    fs::create_dir_all(bin).map_err(|source| io_error("create directory for", bin, source))?;
    let lock_path = bin.join(".uv.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&lock_path)
        .map_err(|source| io_error("open lock for", &lock_path, source))?;
    lock.lock()
        .map_err(|source| io_error("lock", &lock_path, source))?;
    if destination.is_file() {
        return Ok(destination);
    }
    let archive = fetch(asset)?;
    install_verified_uv_archive_with_operations(&archive, asset, bin, operations)
}

fn download_archive(asset: &UvAsset) -> Result<Vec<u8>, UvBootstrapError> {
    download_archive_with_limit(asset, MAX_ARCHIVE_BYTES)
}

fn download_archive_with_limit(asset: &UvAsset, limit: u64) -> Result<Vec<u8>, UvBootstrapError> {
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(60)))
        .build();
    let agent: ureq::Agent = config.into();
    let mut response =
        agent
            .get(&asset.url)
            .call()
            .map_err(|error| UvBootstrapError::Download {
                url: asset.url.clone(),
                reason: error.to_string(),
            })?;
    let archive = response
        .body_mut()
        .with_config()
        .limit(limit)
        .read_to_vec()
        .map_err(|error| UvBootstrapError::Download {
            url: asset.url.clone(),
            reason: error.to_string(),
        })?;
    Ok(archive)
}

/// Verify and atomically install the uv executable from one release archive.
pub fn install_verified_uv_archive(
    archive: &[u8],
    asset: &UvAsset,
    destination_dir: &Path,
) -> Result<PathBuf, UvBootstrapError> {
    let mut operations = SystemUvInstallOperations;
    install_verified_uv_archive_with_operations(archive, asset, destination_dir, &mut operations)
}

trait UvInstallOperations {
    fn write_staged(&mut self, file: &mut File, path: &Path, bytes: &[u8]) -> io::Result<()>;
    fn sync_staged(&mut self, file: &File, path: &Path) -> io::Result<()>;
    fn replace(&mut self, staged: &Path, target: &Path) -> io::Result<()>;
    fn sync_directory(&mut self, path: &Path) -> io::Result<()>;
}

struct SystemUvInstallOperations;

impl UvInstallOperations for SystemUvInstallOperations {
    fn write_staged(&mut self, file: &mut File, _path: &Path, bytes: &[u8]) -> io::Result<()> {
        file.write_all(bytes)
    }

    fn sync_staged(&mut self, file: &File, _path: &Path) -> io::Result<()> {
        file.sync_all()
    }

    fn replace(&mut self, staged: &Path, target: &Path) -> io::Result<()> {
        fs::rename(staged, target)
    }

    fn sync_directory(&mut self, path: &Path) -> io::Result<()> {
        sync_directory(path)
    }
}

fn install_verified_uv_archive_with_operations(
    archive: &[u8],
    asset: &UvAsset,
    destination_dir: &Path,
    operations: &mut impl UvInstallOperations,
) -> Result<PathBuf, UvBootstrapError> {
    let actual = hex_digest(archive);
    if actual != asset.checksum {
        return Err(UvBootstrapError::Checksum {
            expected: asset.checksum.clone(),
            actual,
        });
    }
    let bytes = if asset.filename.ends_with(".zip") {
        executable_from_zip(archive, &asset.executable_name)?
    } else {
        executable_from_tar(archive, &asset.executable_name)?
    };
    fs::create_dir_all(destination_dir)
        .map_err(|source| io_error("create directory for", destination_dir, source))?;
    let target = destination_dir.join(&asset.executable_name);
    let staged = destination_dir.join(format!(".{}.{}.tmp", asset.executable_name, Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&staged)
            .map_err(|source| io_error("create staged", &staged, source))?;
        operations
            .write_staged(&mut file, &staged, &bytes)
            .map_err(|source| io_error("write staged", &staged, source))?;
        set_executable(&file, &staged)?;
        operations
            .sync_staged(&file, &staged)
            .map_err(|source| io_error("sync staged", &staged, source))?;
        operations
            .replace(&staged, &target)
            .map_err(|source| io_error("install", &target, source))?;
        // Best-effort: persist the rename's directory entry too. The staged-file sync
        // already secured the content, so a failure here must not fail an install
        // whose rename landed.
        let _ = operations.sync_directory(destination_dir);
        Ok(target)
    })();
    if result.is_err() {
        let _ = fs::remove_file(staged);
    }
    result
}

fn executable_from_tar(archive: &[u8], name: &str) -> Result<Vec<u8>, UvBootstrapError> {
    let decoder = GzDecoder::new(Cursor::new(archive));
    let mut archive = tar::Archive::new(decoder);
    let entries = archive.entries().map_err(archive_error)?;
    for entry in entries {
        let mut entry = entry.map_err(archive_error)?;
        let path = entry.path().map_err(archive_error)?;
        if entry.header().entry_type().is_file()
            && path.file_name().is_some_and(|item| item == name)
        {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(archive_error)?;
            return Ok(bytes);
        }
    }
    Err(missing_executable(name))
}

fn executable_from_zip(archive: &[u8], name: &str) -> Result<Vec<u8>, UvBootstrapError> {
    let reader = Cursor::new(archive);
    let mut archive = zip::ZipArchive::new(reader).map_err(archive_error)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(archive_error)?;
        if !entry.is_dir()
            && Path::new(entry.name())
                .file_name()
                .is_some_and(|item| item == name)
        {
            let mut bytes = Vec::new();
            entry.read_to_end(&mut bytes).map_err(archive_error)?;
            return Ok(bytes);
        }
    }
    Err(missing_executable(name))
}

fn hex_digest(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn missing_executable(name: &str) -> UvBootstrapError {
    UvBootstrapError::Archive {
        reason: format!("the archive does not contain {name}"),
    }
}

fn archive_error(error: impl std::fmt::Display) -> UvBootstrapError {
    UvBootstrapError::Archive {
        reason: error.to_string(),
    }
}

fn io_error(operation: &'static str, path: &Path, source: io::Error) -> UvBootstrapError {
    UvBootstrapError::Io {
        operation,
        path: path.display().to_string(),
        source,
    }
}

#[cfg(unix)]
fn set_executable(file: &File, path: &Path) -> Result<(), UvBootstrapError> {
    use std::os::unix::fs::PermissionsExt as _;

    let mut permissions = file
        .metadata()
        .map_err(|source| io_error("read permissions for", path, source))?
        .permissions();
    permissions.set_mode(0o755);
    file.set_permissions(permissions)
        .map_err(|source| io_error("set permissions for", path, source))
}

#[cfg(not(unix))]
fn set_executable(_file: &File, _path: &Path) -> Result<(), UvBootstrapError> {
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path).and_then(|directory| directory.sync_all())
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn host_uses_musl() -> bool {
    host_uses_musl_in(Path::new("/lib"))
}

#[cfg(any(target_os = "linux", test))]
fn host_uses_musl_in(lib_dir: &Path) -> bool {
    fs::read_dir(lib_dir).is_ok_and(|entries| {
        entries.filter_map(Result::ok).any(|entry| {
            entry
                .file_name()
                .to_str()
                .is_some_and(|name| name.starts_with("ld-musl-") && name.ends_with(".so.1"))
        })
    })
}

#[cfg(not(target_os = "linux"))]
const fn host_uses_musl() -> bool {
    false
}

#[cfg(test)]
mod private_tests {
    use super::*;
    use flate2::{Compression, write::GzEncoder};
    use std::{net::TcpListener, thread};
    use tempfile::TempDir;

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InstallFault {
        None,
        PartialWriteOnce,
        StagedSync,
        Replace,
        DirectorySync,
    }

    #[derive(Clone, Copy, Debug, Eq, PartialEq)]
    enum InstallEvent {
        Write,
        StagedSync,
        Replace,
        DirectorySync,
    }

    #[derive(Debug)]
    struct RecordingInstallOperations {
        events: Vec<InstallEvent>,
        fault: InstallFault,
        writes: usize,
    }

    impl RecordingInstallOperations {
        fn new(fault: InstallFault) -> Self {
            Self {
                events: Vec::new(),
                fault,
                writes: 0,
            }
        }
    }

    impl UvInstallOperations for RecordingInstallOperations {
        fn write_staged(&mut self, file: &mut File, _path: &Path, bytes: &[u8]) -> io::Result<()> {
            self.events.push(InstallEvent::Write);
            self.writes += 1;
            if self.fault == InstallFault::PartialWriteOnce && self.writes == 1 {
                std::io::Write::write_all(file, &bytes[..bytes.len().min(4)])?;
                return Err(io::Error::other("simulated ENOSPC after a partial write"));
            }
            std::io::Write::write_all(file, bytes)
        }

        fn sync_staged(&mut self, file: &File, _path: &Path) -> io::Result<()> {
            self.events.push(InstallEvent::StagedSync);
            if self.fault == InstallFault::StagedSync {
                return Err(io::Error::other("simulated staged fsync EIO"));
            }
            file.sync_all()
        }

        fn replace(&mut self, staged: &Path, target: &Path) -> io::Result<()> {
            self.events.push(InstallEvent::Replace);
            if self.fault == InstallFault::Replace {
                return Err(io::Error::other("simulated atomic replace failure"));
            }
            fs::rename(staged, target)
        }

        fn sync_directory(&mut self, _path: &Path) -> io::Result<()> {
            self.events.push(InstallEvent::DirectorySync);
            if self.fault == InstallFault::DirectorySync {
                return Err(io::Error::other("simulated directory fsync failure"));
            }
            Ok(())
        }
    }

    fn durability_fixture() -> (Vec<u8>, UvAsset, &'static [u8]) {
        let executable_name = if cfg!(windows) { "uv.exe" } else { "uv" };
        let bytes = b"complete verified uv";
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(encoder);
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(
            &mut header,
            format!("release/{executable_name}"),
            bytes.as_slice(),
        )
        .unwrap();
        let archive = tar.into_inner().unwrap().finish().unwrap();
        let asset = test_asset("https://example.invalid/uv.tar.gz".to_owned(), &archive);
        let asset = UvAsset {
            executable_name: executable_name.to_owned(),
            ..asset
        };
        (archive, asset, bytes)
    }

    fn assert_no_staged_file(directory: &Path, executable_name: &str) {
        let prefix = format!(".{executable_name}.");
        assert!(fs::read_dir(directory).unwrap().all(|entry| {
            let name = entry.unwrap().file_name();
            let name = name.to_string_lossy();
            !(name.starts_with(&prefix) && name.ends_with(".tmp"))
        }));
    }

    #[test]
    fn test_extract_uv_failed_copy_leaves_no_partial_binary() {
        let (archive, asset, _) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::PartialWriteOnce);

        let error = install_verified_uv_archive_with_operations(
            &archive,
            &asset,
            destination.path(),
            &mut operations,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            UvBootstrapError::Io {
                operation: "write staged",
                ..
            }
        ));
        assert!(!destination.path().join(&asset.executable_name).exists());
        assert_no_staged_file(destination.path(), &asset.executable_name);
    }

    #[test]
    fn test_extract_uv_self_heals_after_interrupted_install() {
        let (archive, asset, bytes) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::PartialWriteOnce);

        assert!(
            install_verified_uv_archive_with_operations(
                &archive,
                &asset,
                destination.path(),
                &mut operations,
            )
            .is_err()
        );
        let installed = install_verified_uv_archive_with_operations(
            &archive,
            &asset,
            destination.path(),
            &mut operations,
        )
        .unwrap();

        assert_eq!(fs::read(installed).unwrap(), bytes);
        assert_no_staged_file(destination.path(), &asset.executable_name);
    }

    #[test]
    fn test_extract_uv_fsyncs_staged_file_before_replace() {
        let (archive, asset, bytes) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::None);

        let installed = install_verified_uv_archive_with_operations(
            &archive,
            &asset,
            destination.path(),
            &mut operations,
        )
        .unwrap();

        assert_eq!(fs::read(installed).unwrap(), bytes);
        assert_eq!(
            operations.events,
            [
                InstallEvent::Write,
                InstallEvent::StagedSync,
                InstallEvent::Replace,
                InstallEvent::DirectorySync,
            ]
        );
    }

    #[test]
    fn test_extract_uv_dir_fsync_failure_is_swallowed() {
        let (archive, asset, bytes) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::DirectorySync);

        let installed = install_verified_uv_archive_with_operations(
            &archive,
            &asset,
            destination.path(),
            &mut operations,
        )
        .unwrap();

        assert_eq!(fs::read(installed).unwrap(), bytes);
        assert_no_staged_file(destination.path(), &asset.executable_name);
    }

    #[test]
    fn test_extract_uv_staged_fsync_failure_triggers_existing_cleanup() {
        let (archive, asset, _) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::StagedSync);

        let error = install_verified_uv_archive_with_operations(
            &archive,
            &asset,
            destination.path(),
            &mut operations,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            UvBootstrapError::Io {
                operation: "sync staged",
                ..
            }
        ));
        assert!(!destination.path().join(&asset.executable_name).exists());
        assert_no_staged_file(destination.path(), &asset.executable_name);
    }

    #[test]
    fn test_ensure_uv_downloaded_atomic_install_self_heals() {
        let (archive, asset, bytes) = durability_fixture();
        let root = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::PartialWriteOnce);
        let fetches = std::cell::Cell::new(0);

        assert!(
            ensure_managed_uv_from_asset_with_operations(
                root.path(),
                &asset,
                |_| {
                    fetches.set(fetches.get() + 1);
                    Ok(archive.clone())
                },
                &mut operations,
            )
            .is_err()
        );
        assert!(!managed_uv_path(root.path()).exists());
        let installed = ensure_managed_uv_from_asset_with_operations(
            root.path(),
            &asset,
            |_| {
                fetches.set(fetches.get() + 1);
                Ok(archive.clone())
            },
            &mut operations,
        )
        .unwrap();

        assert_eq!(fetches.get(), 2);
        assert_eq!(installed, managed_uv_path(root.path()));
        assert_eq!(fs::read(installed).unwrap(), bytes);
        assert_no_staged_file(
            managed_uv_path(root.path()).parent().unwrap(),
            &asset.executable_name,
        );
    }

    #[test]
    fn rust_additive_uv_replace_failure_removes_the_verified_staging_file() {
        let (archive, asset, _) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::Replace);

        let error = install_verified_uv_archive_with_operations(
            &archive,
            &asset,
            destination.path(),
            &mut operations,
        )
        .unwrap_err();

        assert!(matches!(
            error,
            UvBootstrapError::Io {
                operation: "install",
                ..
            }
        ));
        assert!(!destination.path().join(&asset.executable_name).exists());
        assert_no_staged_file(destination.path(), &asset.executable_name);
    }

    #[test]
    fn rust_additive_install_operations_start_only_after_checksum_and_archive_acceptance() {
        let (archive, mut asset, _) = durability_fixture();
        let destination = TempDir::new().unwrap();
        let mut operations = RecordingInstallOperations::new(InstallFault::None);
        asset.checksum = "00".repeat(32);

        assert!(matches!(
            install_verified_uv_archive_with_operations(
                &archive,
                &asset,
                destination.path(),
                &mut operations,
            ),
            Err(UvBootstrapError::Checksum { .. })
        ));
        assert!(operations.events.is_empty());

        let invalid_archive = b"not a release archive";
        let mut invalid_asset = test_asset(
            "https://example.invalid/uv.tar.gz".to_owned(),
            invalid_archive,
        );
        invalid_asset
            .executable_name
            .clone_from(&asset.executable_name);
        assert!(matches!(
            install_verified_uv_archive_with_operations(
                invalid_archive,
                &invalid_asset,
                destination.path(),
                &mut operations,
            ),
            Err(UvBootstrapError::Archive { .. })
        ));
        assert!(operations.events.is_empty());
        assert!(!destination.path().join(&asset.executable_name).exists());
    }

    #[test]
    fn test_is_musl_true_when_ld_musl_present() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("ld-musl-x86_64.so.1"), b"").unwrap();

        assert!(host_uses_musl_in(root.path()));
    }

    #[test]
    fn test_is_musl_false_when_ld_musl_absent() {
        let root = TempDir::new().unwrap();
        fs::write(root.path().join("ld-linux-x86-64.so.2"), b"").unwrap();

        assert!(!host_uses_musl_in(root.path()));
    }

    #[test]
    fn test_is_musl_false_when_lib_dir_missing() {
        let root = TempDir::new().unwrap();

        assert!(!host_uses_musl_in(&root.path().join("missing")));
    }

    #[test]
    fn unpinned_triple_fails_closed_with_a_typed_error() {
        // uvman.py:229-233: a triple with no pinned hash refuses with a typed error
        // naming the triple, never an unverified download. Every producible triple
        // is pinned, so only this white-box construction can reach the branch.
        let martian = UvTarget {
            triple: "riscv64-unknown-linux-gnu".to_owned(),
            windows: false,
        };
        let error = uv_asset(&martian, None).unwrap_err();
        assert!(matches!(
            &error,
            UvBootstrapError::NoPinnedChecksum { triple } if triple == "riscv64-unknown-linux-gnu"
        ));
        assert!(error.to_string().contains("riscv64-unknown-linux-gnu"));
    }

    #[test]
    fn checksum_table_pins_exactly_the_producible_triples() {
        // test_uv_sha256_covers_every_producible_triple (tests/test_uvman.py): the
        // pinned table keys on exactly the triples from_parts can emit — no stale
        // orphan pin, no reachable platform without a hash to verify against. The
        // integration ports prove the producible -> pinned direction through
        // uv_asset; CHECKSUMS is private, so the reverse direction lives here.
        let produced: std::collections::BTreeSet<String> = [
            ("x86_64", "linux", false),
            ("aarch64", "linux", false),
            ("x86_64", "linux", true),
            ("aarch64", "linux", true),
            ("x86_64", "macos", false),
            ("aarch64", "macos", false),
            ("x86_64", "windows", false),
            ("aarch64", "windows", false),
        ]
        .iter()
        .map(|(arch, os, musl)| UvTarget::from_parts(arch, os, *musl).unwrap().triple)
        .collect();
        assert_eq!(produced.len(), 8);
        let pinned: std::collections::BTreeSet<String> = CHECKSUMS
            .iter()
            .map(|(triple, _)| (*triple).to_owned())
            .collect();
        assert_eq!(pinned, produced);
        // Each pinned value is a 64-character lowercase-hex SHA-256 digest.
        assert!(CHECKSUMS.iter().all(|(_, hash)| {
            hash.len() == 64
                && hash
                    .chars()
                    .all(|character| character.is_ascii_digit() || ('a'..='f').contains(&character))
        }));
    }

    fn tar_archive() -> Vec<u8> {
        let encoder = GzEncoder::new(Vec::new(), Compression::default());
        let mut tar = tar::Builder::new(encoder);
        let bytes = b"private uv";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "release/uv", bytes.as_slice())
            .unwrap();
        tar.into_inner().unwrap().finish().unwrap()
    }

    fn test_asset(url: String, archive: &[u8]) -> UvAsset {
        UvAsset {
            version: "test".to_owned(),
            filename: "uv-test.tar.gz".to_owned(),
            url,
            checksum: hex_digest(archive),
            executable_name: "uv".to_owned(),
        }
    }

    fn one_response(body: Vec<u8>) -> (String, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let handle = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )
            .unwrap();
            stream.write_all(&body).unwrap();
        });
        (format!("http://{address}/archive"), handle)
    }

    #[test]
    fn downloader_handles_success_size_limits_and_connection_failures() {
        let (url, server) = one_response(b"archive".to_vec());
        let asset = test_asset(url, b"archive");
        assert_eq!(download_archive(&asset).unwrap(), b"archive");
        server.join().unwrap();

        let (url, server) = one_response(vec![b'x'; 64]);
        let asset = test_asset(url, &[b'x'; 64]);
        assert!(matches!(
            download_archive_with_limit(&asset, 8),
            Err(UvBootstrapError::Download { .. })
        ));
        server.join().unwrap();

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        let asset = test_asset(format!("http://{address}/archive"), b"");
        assert!(matches!(
            download_archive_with_limit(&asset, 8),
            Err(UvBootstrapError::Download { .. })
        ));
    }

    #[test]
    fn injectable_bootstrap_uses_the_same_lock_verify_and_install_path() {
        let archive = tar_archive();
        let asset = test_asset("https://example.invalid/archive".to_owned(), &archive);
        let root = TempDir::new().unwrap();
        let installed =
            ensure_managed_uv_from_asset(root.path(), &asset, |_| Ok(archive.clone())).unwrap();
        assert_eq!(fs::read(&installed).unwrap(), b"private uv");
        assert_eq!(
            ensure_managed_uv_from_asset(root.path(), &asset, |_| panic!("must not fetch"))
                .unwrap(),
            installed
        );

        let failed = TempDir::new().unwrap();
        assert!(matches!(
            ensure_managed_uv_from_asset(failed.path(), &asset, |_| {
                Err(UvBootstrapError::Download {
                    url: asset.url.clone(),
                    reason: "offline".to_owned(),
                })
            }),
            Err(UvBootstrapError::Download { .. })
        ));
    }
}
