use std::{fs, io::Write as _};

use flate2::{Compression, write::GzEncoder};
use sha2::{Digest as _, Sha256};
use skit_runtime::{UvAsset, UvTarget, install_verified_uv_archive, uv_asset};
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
