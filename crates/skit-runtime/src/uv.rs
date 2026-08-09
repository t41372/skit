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
    #[error("the downloaded uv archive failed checksum verification")]
    Checksum,
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
            Self::Checksum => {
                Message::new("the downloaded uv archive failed checksum verification")
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
#[must_use]
pub fn uv_asset(target: &UvTarget, mirror_base: Option<&str>) -> UvAsset {
    let extension = if target.windows { "zip" } else { "tar.gz" };
    let filename = format!("uv-{}.{extension}", target.triple);
    let base = mirror_base.unwrap_or(OFFICIAL_BASE).trim_end_matches('/');
    let checksum = CHECKSUMS
        .iter()
        .find_map(|(triple, checksum)| (*triple == target.triple).then_some(*checksum))
        .expect("each supported target has a pinned checksum");
    UvAsset {
        version: UV_VERSION.to_owned(),
        url: format!("{base}/{UV_VERSION}/{filename}"),
        filename,
        checksum: checksum.to_owned(),
        executable_name: if target.windows { "uv.exe" } else { "uv" }.to_owned(),
    }
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
    let asset = uv_asset(&UvTarget::current()?, mirror_base);
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
    install_verified_uv_archive(&archive, asset, bin)
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
    if hex_digest(archive) != asset.checksum {
        return Err(UvBootstrapError::Checksum);
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
        file.write_all(&bytes)
            .map_err(|source| io_error("write staged", &staged, source))?;
        set_executable(&file, &staged)?;
        file.sync_all()
            .map_err(|source| io_error("sync staged", &staged, source))?;
        fs::rename(&staged, &target).map_err(|source| io_error("install", &target, source))?;
        sync_directory(destination_dir)?;
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
fn sync_directory(path: &Path) -> Result<(), UvBootstrapError> {
    File::open(path)
        .and_then(|directory| directory.sync_all())
        .map_err(|source| io_error("sync directory", path, source))
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> Result<(), UvBootstrapError> {
    Ok(())
}

#[cfg(target_os = "linux")]
fn host_uses_musl() -> bool {
    fs::read_dir("/lib").is_ok_and(|entries| {
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
