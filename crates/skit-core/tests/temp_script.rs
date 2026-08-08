use std::fs;

use skit_core::materialize_temp_script;

#[test]
fn injected_temp_script_is_private_ephemeral_and_byte_exact()
-> Result<(), Box<dyn std::error::Error>> {
    let content = "TOKEN = \"secret\"\nprint(TOKEN)\n";
    let temp_dir = std::env::temp_dir();
    let path;
    {
        let guard = materialize_temp_script(content, ".py")?;
        path = guard.path().to_owned();
        assert_eq!(path.parent(), Some(temp_dir.as_path()));
        assert_eq!(path.extension().and_then(|value| value.to_str()), Some("py"));
        assert_eq!(fs::read(&path)?, content.as_bytes());

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::metadata(&path)?.permissions().mode() & 0o777;
            assert_eq!(mode, 0o600);
        }
    }
    assert!(!path.exists());
    Ok(())
}

#[test]
fn suffix_without_dot_is_normalized() -> Result<(), Box<dyn std::error::Error>> {
    let guard = materialize_temp_script("print('ok')\n", "py")?;
    assert_eq!(
        guard.path().extension().and_then(|value| value.to_str()),
        Some("py")
    );
    Ok(())
}
