//! Resolve entry payload paths from the v0.4 data layout, and the host directories that
//! layout sits in.

use std::{
    borrow::Cow,
    env,
    ffi::OsString,
    fs,
    path::{Path, PathBuf},
};

use skit_application::{RepositoryError, canonical_stored_filename};
use skit_domain::{Entry, Slug, StorageMode};
use skit_i18n::Message;

use crate::FileStore;

/// Expand the current user's leading `~` with the platform home-directory adapter.
///
/// Other text stays byte-for-byte unchanged. Environment-variable expansion is not part of this
/// path contract.
#[must_use]
pub fn expand_user_path(path: &Path) -> PathBuf {
    path.to_str().map_or_else(
        || path.to_path_buf(),
        |value| PathBuf::from(expand_leading_tilde(value).as_ref()),
    )
}

/// Expand a leading `~` with the home directory this host names.
#[cfg(not(windows))]
fn expand_leading_tilde(value: &str) -> Cow<'_, str> {
    shellexpand::tilde(value)
}

/// Expand a leading `~` the way version 0.4 expands it on this host.
///
/// Version 0.4 expands with CPython's `os.path.expanduser`. Its Windows form reads USERPROFILE, and
/// then HOMEDRIVE together with HOMEPATH, from the environment. The default adapter asks the shell
/// for the profile folder instead, and no environment can redirect that answer, so a caller that
/// names a home directory was ignored. Read what version 0.4 reads first, and keep the shell answer
/// as the last resort.
#[cfg(windows)]
fn expand_leading_tilde(value: &str) -> Cow<'_, str> {
    windows_home().map_or_else(
        || shellexpand::tilde(value),
        |home| shellexpand::tilde_with_context(value, || Some(home)),
    )
}

/// The home directory the environment names, in the order version 0.4 reads it.
#[cfg(windows)]
fn windows_home() -> Option<String> {
    fn named(variable: &str) -> Option<String> {
        std::env::var(variable)
            .ok()
            .filter(|value| !value.is_empty())
    }

    named("USERPROFILE").or_else(|| Some(format!("{}{}", named("HOMEDRIVE")?, named("HOMEPATH")?)))
}

/// The directory name skit appends to every host root.
const APPLICATION: &str = "skit";

/// Name one of skit's host directories.
///
/// A redirect variable wins when the host sets one. Otherwise the directory sits below the home
/// directory at the parts this host uses, and a host that names neither has no directory. The
/// values arrive as parameters, so an owner can ask for every answer without changing the
/// environment of the whole process.
fn platform_directory(
    redirect: Option<OsString>,
    home: Option<OsString>,
    home_parts: &[&str],
) -> Option<PathBuf> {
    redirect
        .map(|redirect| PathBuf::from(redirect).join(APPLICATION))
        .or_else(|| {
            home.map(|home| {
                let mut path = PathBuf::from(home);
                path.extend(home_parts);
                path.push(APPLICATION);
                path
            })
        })
}

/// Name skit's data directory: `$XDG_DATA_HOME`, or `~/.local/share`.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub fn platform_data_dir() -> Option<PathBuf> {
    platform_directory(
        env::var_os("XDG_DATA_HOME"),
        env::var_os("HOME"),
        &[".local", "share"],
    )
}

/// Name skit's state directory: `$XDG_STATE_HOME`, or `~/.local/state`.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub fn platform_state_dir() -> Option<PathBuf> {
    platform_directory(
        env::var_os("XDG_STATE_HOME"),
        env::var_os("HOME"),
        &[".local", "state"],
    )
}

/// Name skit's configuration directory: `$XDG_CONFIG_HOME`, or `~/.config`.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub fn platform_config_dir() -> Option<PathBuf> {
    platform_directory(
        env::var_os("XDG_CONFIG_HOME"),
        env::var_os("HOME"),
        &[".config"],
    )
}

/// Name skit's data directory below the macOS application-support directory.
#[cfg(target_os = "macos")]
#[must_use]
pub fn platform_data_dir() -> Option<PathBuf> {
    platform_directory(
        None,
        env::var_os("HOME"),
        &["Library", "Application Support"],
    )
}

/// Name skit's state directory. macOS keeps state with the data.
#[cfg(target_os = "macos")]
#[must_use]
pub fn platform_state_dir() -> Option<PathBuf> {
    platform_data_dir()
}

/// Name skit's configuration directory. macOS keeps the configuration with the data.
#[cfg(target_os = "macos")]
#[must_use]
pub fn platform_config_dir() -> Option<PathBuf> {
    platform_data_dir()
}

/// Name skit's data directory below `%LOCALAPPDATA%`, or below `%APPDATA%`.
#[cfg(windows)]
#[must_use]
pub fn platform_data_dir() -> Option<PathBuf> {
    platform_directory(
        env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA")),
        None,
        &[],
    )
}

/// Name skit's state directory. Windows keeps state with the data.
#[cfg(windows)]
#[must_use]
pub fn platform_state_dir() -> Option<PathBuf> {
    platform_data_dir()
}

/// Name skit's configuration directory below `%APPDATA%`.
#[cfg(windows)]
#[must_use]
pub fn platform_config_dir() -> Option<PathBuf> {
    platform_directory(env::var_os("APPDATA"), None, &[])
}

/// Name skit's data directory. A host that is neither Unix nor Windows names none.
#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn platform_data_dir() -> Option<PathBuf> {
    None
}

/// Name skit's state directory. A host that is neither Unix nor Windows names none.
#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn platform_state_dir() -> Option<PathBuf> {
    None
}

/// Name skit's configuration directory. A host that is neither Unix nor Windows names none.
#[cfg(not(any(unix, windows)))]
#[must_use]
pub fn platform_config_dir() -> Option<PathBuf> {
    None
}

/// Return the stored copy name for a known entry kind.
#[must_use]
pub fn stored_filename(kind: &str) -> Option<&'static str> {
    canonical_stored_filename(kind)
}

/// Return all payload filenames accepted for one known entry kind.
#[must_use]
pub fn stored_filenames(kind: &str) -> &'static [&'static str] {
    match kind {
        "js" => &["script.js", "script.mjs", "script.cjs"],
        "ts" => &["script.ts", "script.mts", "script.cts"],
        "python" => &["script.py"],
        "shell" => &["script.sh"],
        "fish" => &["script.fish"],
        "powershell" => &["script.ps1"],
        "ruby" => &["script.rb"],
        "perl" => &["script.pl"],
        "lua" => &["script.lua"],
        "r" => &["script.r"],
        "prompt" => &["prompt.md"],
        "exe" | "command" => &[],
        _ => &["payload"],
    }
}

impl FileStore {
    /// Return the directory that owns one entry.
    #[must_use]
    pub fn entry_dir_path(&self, slug: &Slug) -> PathBuf {
        self.data_dir().join("scripts").join(slug.as_str())
    }

    /// Return the current launch or edit payload path.
    pub fn payload_path(&self, entry: &Entry) -> Result<PathBuf, RepositoryError> {
        if entry.meta.kind.as_str() == "exe" || entry.meta.mode == StorageMode::Reference {
            return Ok(PathBuf::from(&entry.meta.source));
        }

        let directory = self.entry_dir_path(&entry.slug);
        for name in stored_filenames(entry.meta.kind.as_str()) {
            let path = directory.join(name);
            if path.is_file() {
                return Ok(path);
            }
        }

        let reader =
            fs::read_dir(&directory).map_err(|error| io_error("scan", &directory, error))?;
        let mut files = Vec::new();
        for item in reader {
            let item = item.map_err(|error| io_error("scan", &directory, error))?;
            if is_support_file(&item.file_name().to_string_lossy()) {
                continue;
            }
            let file_type = item
                .file_type()
                .map_err(|error| io_error("inspect", &item.path(), error))?;
            if file_type.is_file() {
                files.push(item.path());
            }
        }
        files.sort();
        match files.as_slice() {
            [path] => Ok(path.clone()),
            [] => Err(RepositoryError::InvalidMutation {
                reason: Message::new("copy entry has no stored payload"),
            }),
            _ => Err(RepositoryError::InvalidMutation {
                reason: Message::new("copy entry has more than one possible stored payload"),
            }),
        }
    }
}

/// Whether skit itself owns this name inside an entry directory.
///
/// A stored payload is never one of these, so the payload scan must skip them. Missing one
/// makes a private file look like a second payload and blocks every launch of that entry.
pub(crate) fn is_support_file(name: &str) -> bool {
    matches!(
        name,
        "meta.toml" | "package.json" | "package-lock.json" | "bun.lock" | "bun.lockb" | "deno.lock"
    )
        // the dependency stamp, its crash backup, and its staging directories
        || name.starts_with(".skit-deps")
        // one run's staged injected source
        || name.starts_with(".run-")
        || name.starts_with(".injected-")
        // an atomic replacement sibling left by an interrupted write
        || (name.starts_with('.') && name.ends_with(".tmp"))
}

fn io_error(
    operation: &'static str,
    path: &std::path::Path,
    error: std::io::Error,
) -> RepositoryError {
    RepositoryError::Io {
        operation,
        path: path.display().to_string(),
        reason: error.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every answer the shared rule can give, without asking this host what it holds.
    ///
    /// The redirect wins, the home directory answers next at the parts the caller names, and a
    /// caller that holds neither gets nothing.
    #[test]
    fn a_platform_directory_prefers_a_redirect_then_the_home_directory() {
        let redirect = Some(OsString::from("/redirect"));
        let home = Some(OsString::from("/home/user"));

        assert_eq!(
            platform_directory(redirect.clone(), home.clone(), &[".local", "share"]),
            Some(PathBuf::from("/redirect/skit"))
        );
        assert_eq!(
            platform_directory(None, home.clone(), &[".local", "share"]),
            Some(PathBuf::from("/home/user/.local/share/skit"))
        );
        assert_eq!(
            platform_directory(None, home.clone(), &[".config"]),
            Some(PathBuf::from("/home/user/.config/skit"))
        );
        assert_eq!(
            platform_directory(None, home, &["Library", "Application Support"]),
            Some(PathBuf::from("/home/user/Library/Application Support/skit"))
        );
        // A root that needs no home directory keeps the root, and holds no parts.
        assert_eq!(
            platform_directory(redirect, None, &[]),
            Some(PathBuf::from("/redirect/skit"))
        );
        assert_eq!(platform_directory(None, None, &[".local", "share"]), None);
    }

    /// Each directory reads the variables its own host names, and falls back where it should.
    ///
    /// The answer is compared with the shared rule over the same values, so a directory that
    /// starts reading another variable, or that loses its fallback parts, fails here.
    #[test]
    #[cfg(all(unix, not(target_os = "macos")))]
    fn each_platform_directory_reads_the_variables_this_host_names() {
        let home = env::var_os("HOME");

        assert_eq!(
            platform_data_dir(),
            platform_directory(
                env::var_os("XDG_DATA_HOME"),
                home.clone(),
                &[".local", "share"]
            )
        );
        assert_eq!(
            platform_state_dir(),
            platform_directory(
                env::var_os("XDG_STATE_HOME"),
                home.clone(),
                &[".local", "state"]
            )
        );
        assert_eq!(
            platform_config_dir(),
            platform_directory(env::var_os("XDG_CONFIG_HOME"), home, &[".config"])
        );
    }

    /// macOS keeps state and configuration with the data, below one application-support root.
    #[test]
    #[cfg(target_os = "macos")]
    fn each_platform_directory_reads_the_variables_this_host_names() {
        let expected = platform_directory(
            None,
            env::var_os("HOME"),
            &["Library", "Application Support"],
        );

        assert_eq!(platform_data_dir(), expected);
        assert_eq!(platform_state_dir(), expected);
        assert_eq!(platform_config_dir(), expected);
    }

    /// Windows keeps state with the data below the local root, and the configuration below the
    /// roaming root.
    #[test]
    #[cfg(windows)]
    fn each_platform_directory_reads_the_variables_this_host_names() {
        let data = platform_directory(
            env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA")),
            None,
            &[],
        );

        assert_eq!(platform_data_dir(), data);
        assert_eq!(platform_state_dir(), data);
        assert_eq!(
            platform_config_dir(),
            platform_directory(env::var_os("APPDATA"), None, &[])
        );
    }
}
