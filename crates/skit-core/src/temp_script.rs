use std::env;
use std::error::Error as StdError;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP_ID: AtomicU64 = AtomicU64::new(0);
const MAX_ATTEMPTS: u64 = 128;

/// An injected source snapshot that is deleted when its guard is dropped.
#[derive(Debug)]
pub struct TempScript {
    path: PathBuf,
}

impl TempScript {
    /// The exact path a launch snapshot must consume while this guard stays alive.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempScript {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

/// Failure to materialize an injected source snapshot in the OS temp directory.
#[derive(Debug)]
pub struct TempScriptError {
    pub path: PathBuf,
    pub source: io::Error,
}

impl fmt::Display for TempScriptError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "cannot create injected temporary script {}: {}",
            self.path.display(),
            self.source
        )
    }
}

impl StdError for TempScriptError {
    fn source(&self) -> Option<&(dyn StdError + 'static)> {
        Some(&self.source)
    }
}

/// Write one injected source snapshot as a private create-new file in the OS temp dir.
///
/// The function never falls back beside the stored/original script: injected text may
/// contain plaintext secret values, so a failure to obtain a private temporary file is
/// a launch refusal, not permission to leave a durable copy behind.
///
/// # Errors
///
/// Returns the final create/write error after bounded collision retries.
pub fn materialize_temp_script(text: &str, suffix: &str) -> Result<TempScript, TempScriptError> {
    let directory = env::temp_dir();
    let process = std::process::id();
    let suffix = if suffix.starts_with('.') {
        suffix.to_owned()
    } else if suffix.is_empty() {
        String::new()
    } else {
        format!(".{suffix}")
    };

    let mut last_error = None;
    let mut last_path = directory.join(format!("skit-{process}-injected{suffix}"));
    for _ in 0..MAX_ATTEMPTS {
        let id = NEXT_TEMP_ID.fetch_add(1, Ordering::Relaxed);
        let path = directory.join(format!("skit-{process}-{id}-injected{suffix}"));
        last_path = path.clone();
        match private_create(&path) {
            Ok(mut file) => {
                if let Err(source) = file.write_all(text.as_bytes()) {
                    let _ = fs::remove_file(&path);
                    return Err(TempScriptError { path, source });
                }
                return Ok(TempScript { path });
            }
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => {
                last_error = Some(source);
            }
            Err(source) => return Err(TempScriptError { path, source }),
        }
    }
    Err(TempScriptError {
        path: last_path,
        source: last_error.unwrap_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, "temporary filename collision")),
    })
}

#[cfg(unix)]
fn private_create(path: &Path) -> io::Result<fs::File> {
    use std::os::unix::fs::OpenOptionsExt;

    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(not(unix))]
fn private_create(path: &Path) -> io::Result<fs::File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}
