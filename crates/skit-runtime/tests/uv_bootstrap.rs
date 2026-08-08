use std::{
    fs,
    io::{Cursor, Write as _},
};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest as _, Sha256};
use skit_runtime::{
    UvAsset, UvBootstrapError, UvTarget, ensure_managed_uv, install_verified_uv_archive,
    managed_uv_path, uv_asset,
};
use tempfile::TempDir;

#[test]
fn current_uv_assets_cover_the_supported_v040_platforms_and_mirrors() {
    let linux = UvTarget::from_parts("x86_64", "linux", false).unwrap();
    let musl = UvTarget::from_parts("aarch64", "linux", true).unwrap();
    let mac = UvTarget::from_parts("aarch64", "macos", false).unwrap();
    let windows = UvTarget::from_parts("x86_64", "windows", false).unwrap();

    assert_eq!(linux.triple(), "x86_64-unknown-linux-gnu");
    assert_eq!(musl.triple(), "aarch64-unknown-linux-musl");
    assert_eq!(mac.triple(), "aarch64-apple-darwin");
    assert_eq!(windows.triple(), "x86_64-pc-windows-msvc");
    assert!(UvTarget::from_parts("riscv64", "linux", false).is_err());

    let official = uv_asset(&linux, None);
    assert_eq!(official.version, "0.12.3");
    assert_eq!(
        official.url,
        "https://github.com/astral-sh/uv/releases/download/0.12.3/uv-x86_64-unknown-linux-gnu.tar.gz"
    );
    assert_eq!(
        official.checksum,
        "600cf9a742aca00d292673b16b5acffaa7b8c269a364ad0c2e79498dcb1fe101"
    );
    assert!(uv_asset(&windows, None).filename.ends_with(".zip"));
    assert!(
        uv_asset(&linux, Some("https://mirror.example/astral-sh/uv"))
            .url
            .starts_with("https://mirror.example/astral-sh/uv/0.12.3/")
    );
}

#[test]
fn a_verified_tar_archive_installs_only_the_uv_binary_atomically() {
    let mut archive = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut archive);
        let bytes = b"uv-test-binary";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "uv-test/uv", bytes.as_slice())
            .unwrap();
        tar.finish().unwrap();
    }
    archive.flush().unwrap();
    let archive = archive.finish().unwrap();
    let checksum = Sha256::digest(&archive)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    let asset = UvAsset {
        version: "test".to_owned(),
        filename: "uv-test.tar.gz".to_owned(),
        url: "https://example.invalid/uv-test.tar.gz".to_owned(),
        checksum,
        executable_name: "uv".to_owned(),
    };
    let root = TempDir::new().unwrap();

    let installed = install_verified_uv_archive(&archive, &asset, root.path()).unwrap();

    assert_eq!(installed, root.path().join("uv"));
    assert_eq!(fs::read(installed).unwrap(), b"uv-test-binary");
    assert!(
        install_verified_uv_archive(b"tampered", &asset, root.path()).is_err(),
        "a checksum mismatch must fail closed"
    );
}

fn asset_for(filename: &str, executable_name: &str, archive: &[u8]) -> UvAsset {
    UvAsset {
        version: "test".to_owned(),
        filename: filename.to_owned(),
        url: "https://example.invalid/archive".to_owned(),
        checksum: Sha256::digest(archive)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
        executable_name: executable_name.to_owned(),
    }
}

#[test]
fn zip_and_invalid_archives_are_handled_without_extracting_other_files() {
    let mut archive = zip::ZipWriter::new(Cursor::new(Vec::new()));
    archive
        .start_file("uv-test/uv.exe", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(b"windows-test-binary").unwrap();
    archive
        .start_file(
            "uv-test/ignored.txt",
            zip::write::SimpleFileOptions::default(),
        )
        .unwrap();
    archive.write_all(b"ignored").unwrap();
    let archive = archive.finish().unwrap().into_inner();
    let asset = asset_for("uv-test.zip", "uv.exe", &archive);
    let root = TempDir::new().unwrap();
    let installed = install_verified_uv_archive(&archive, &asset, root.path()).unwrap();
    assert_eq!(fs::read(installed).unwrap(), b"windows-test-binary");
    assert!(!root.path().join("ignored.txt").exists());

    let missing = asset_for("uv-test.zip", "missing.exe", &archive);
    assert!(matches!(
        install_verified_uv_archive(&archive, &missing, root.path()),
        Err(UvBootstrapError::Archive { .. })
    ));
    let malformed = asset_for("uv-test.zip", "uv.exe", b"not a zip");
    assert!(matches!(
        install_verified_uv_archive(b"not a zip", &malformed, root.path()),
        Err(UvBootstrapError::Archive { .. })
    ));
    let malformed_tar = asset_for("uv-test.tar.gz", "uv", b"not a gzip stream");
    assert!(matches!(
        install_verified_uv_archive(b"not a gzip stream", &malformed_tar, root.path()),
        Err(UvBootstrapError::Archive { .. })
    ));

    let mut tar_bytes = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut tar_bytes);
        let bytes = b"ignored";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o644);
        header.set_cksum();
        tar.append_data(&mut header, "release/ignored.txt", bytes.as_slice())
            .unwrap();
        tar.finish().unwrap();
    }
    tar_bytes.flush().unwrap();
    let tar_bytes = tar_bytes.finish().unwrap();
    let missing_tar = asset_for("uv-test.tar.gz", "uv", &tar_bytes);
    assert!(matches!(
        install_verified_uv_archive(&tar_bytes, &missing_tar, root.path()),
        Err(UvBootstrapError::Archive { .. })
    ));
}

#[test]
fn target_and_managed_path_failures_are_typed() {
    assert!(UvTarget::from_parts("x86_64", "future-os", false).is_err());
    assert!(UvTarget::current().is_ok());

    let root = TempDir::new().unwrap();
    let expected = root
        .path()
        .join("bin")
        .join(if cfg!(windows) { "uv.exe" } else { "uv" });
    assert_eq!(managed_uv_path(root.path()), expected);
    fs::create_dir_all(expected.parent().unwrap()).unwrap();
    fs::write(&expected, "existing").unwrap();
    assert_eq!(ensure_managed_uv(root.path(), None).unwrap(), expected);

    let blocker = root.path().join("blocker");
    fs::write(&blocker, "file").unwrap();
    assert!(matches!(
        ensure_managed_uv(&blocker, None),
        Err(UvBootstrapError::Io { .. })
    ));
}

#[test]
fn failed_atomic_install_removes_its_staged_file() {
    let mut archive = GzEncoder::new(Vec::new(), Compression::default());
    {
        let mut tar = tar::Builder::new(&mut archive);
        let bytes = b"uv-test-binary";
        let mut header = tar::Header::new_gnu();
        header.set_size(bytes.len() as u64);
        header.set_mode(0o755);
        header.set_cksum();
        tar.append_data(&mut header, "uv-test/uv", bytes.as_slice())
            .unwrap();
        tar.finish().unwrap();
    }
    archive.flush().unwrap();
    let archive = archive.finish().unwrap();
    let asset = asset_for("uv-test.tar.gz", "uv", &archive);
    let root = TempDir::new().unwrap();
    fs::create_dir(root.path().join("uv")).unwrap();

    assert!(matches!(
        install_verified_uv_archive(&archive, &asset, root.path()),
        Err(UvBootstrapError::Io { .. })
    ));
    assert!(fs::read_dir(root.path()).unwrap().all(|entry| {
        !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .ends_with(".tmp")
    }));
}
