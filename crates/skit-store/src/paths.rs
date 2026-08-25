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

/// Read the root a redirect variable names.
///
/// Version 0.4 removes the spaces around the value and treats a blank one as unset, then uses the
/// value without those spaces (`platformdirs/_xdg.py:16,35,61`). A value that is not Unicode holds
/// no spaces this rule can remove, and it still names a real directory, so it stays as it is. A
/// blank value names no directory on any host.
fn redirect_root(value: Option<OsString>) -> Option<PathBuf> {
    let value = value?;
    match value.to_str() {
        Some(text) => {
            let named = text.trim();
            (!named.is_empty()).then(|| PathBuf::from(named))
        }
        None => Some(PathBuf::from(value)),
    }
}

/// Name one of skit's host directories.
///
/// A redirect variable wins when the host sets one. Otherwise the directory sits below the home
/// directory at the parts this host uses, and a host that names neither has no directory. Every
/// root then carries the parts this host keeps below it. The values arrive as parameters, so an
/// owner can ask for every answer without changing the environment of the whole process.
fn platform_directory(
    redirect: Option<OsString>,
    home: Option<OsString>,
    home_parts: &[&str],
    application_parts: &[&str],
) -> Option<PathBuf> {
    let mut path = match redirect_root(redirect) {
        Some(root) => root,
        None => {
            let mut path = PathBuf::from(home.filter(|home| !home.is_empty())?);
            path.extend(home_parts);
            path
        }
    };
    path.extend(application_parts);
    Some(path)
}

/// Read the directory an override variable names.
///
/// Version 0.4 reads `SKIT_DATA_DIR`, `SKIT_STATE_DIR`, and `SKIT_CONFIG_DIR` as plain text, and it
/// keeps a value that holds anything (`skit-oracle/src/skit/paths.py:16-29`). An empty value names
/// no directory, so the host directory answers instead. Version 0.4 keeps the spaces inside a value
/// that holds them, so this rule leaves them there.
#[must_use]
pub fn override_directory(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// Name skit's data directory: `$XDG_DATA_HOME`, or `~/.local/share`.
#[cfg(all(unix, not(target_os = "macos")))]
#[must_use]
pub fn platform_data_dir() -> Option<PathBuf> {
    platform_directory(
        env::var_os("XDG_DATA_HOME"),
        env::var_os("HOME"),
        &[".local", "share"],
        &[APPLICATION],
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
        &[APPLICATION],
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
        &[APPLICATION],
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
        &[APPLICATION],
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
///
/// Version 0.4 reads the local root, and it names `skit` twice below that root. Its directory
/// adapter puts an author folder before the application folder, and the author defaults to the
/// application name (`platformdirs/windows.py:29-33,36-44`), so a version 0.4 library on Windows
/// sits at `%LOCALAPPDATA%\skit\skit`. That is where the data of a user who upgrades already is.
/// Version 0.4 stops when the local root is absent; skit reads the roaming root then, which keeps
/// a host that names only one root usable.
#[cfg(windows)]
#[must_use]
pub fn platform_data_dir() -> Option<PathBuf> {
    platform_directory(
        env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA")),
        None,
        &[],
        &[APPLICATION, APPLICATION],
    )
}

/// Name skit's state directory. Windows keeps state with the data.
#[cfg(windows)]
#[must_use]
pub fn platform_state_dir() -> Option<PathBuf> {
    platform_data_dir()
}

/// Name skit's configuration directory. Windows keeps the configuration with the data.
///
/// Version 0.4 asks its directory adapter for a configuration directory, and the Windows form of
/// that adapter answers with the data directory (`platformdirs/windows.py:59-61`).
#[cfg(windows)]
#[must_use]
pub fn platform_config_dir() -> Option<PathBuf> {
    platform_data_dir()
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
        let application = &[APPLICATION];

        assert_eq!(
            platform_directory(
                redirect.clone(),
                home.clone(),
                &[".local", "share"],
                application
            ),
            Some(PathBuf::from("/redirect/skit"))
        );
        assert_eq!(
            platform_directory(None, home.clone(), &[".local", "share"], application),
            Some(PathBuf::from("/home/user/.local/share/skit"))
        );
        assert_eq!(
            platform_directory(None, home.clone(), &[".config"], application),
            Some(PathBuf::from("/home/user/.config/skit"))
        );
        assert_eq!(
            platform_directory(None, home, &["Library", "Application Support"], application),
            Some(PathBuf::from("/home/user/Library/Application Support/skit"))
        );
        // A root that needs no home directory keeps the root, and holds no parts.
        assert_eq!(
            platform_directory(redirect, None, &[], application),
            Some(PathBuf::from("/redirect/skit"))
        );
        assert_eq!(
            platform_directory(None, None, &[".local", "share"], application),
            None
        );
        // Version 0.4 names the application twice below a Windows root
        // (`platformdirs/windows.py:36-44`).
        assert_eq!(
            platform_directory(
                Some(OsString::from(r"C:\Users\u\AppData\Local")),
                None,
                &[],
                &[APPLICATION, APPLICATION]
            ),
            Some(
                PathBuf::from(r"C:\Users\u\AppData\Local")
                    .join("skit")
                    .join("skit")
            )
        );
    }

    /// A blank value names no directory, and a value with spaces around it names the same
    /// directory as the value without them.
    ///
    /// Version 0.4 removes those spaces and treats what remains as the answer, so a variable that
    /// holds only spaces leaves the home directory to answer (`platformdirs/_xdg.py:16,35,61`).
    #[test]
    fn a_blank_redirect_leaves_the_home_directory_to_answer() {
        let home = Some(OsString::from("/home/user"));
        let application = &[APPLICATION];

        for blank in ["", "   ", "\t"] {
            assert_eq!(
                platform_directory(
                    Some(OsString::from(blank)),
                    home.clone(),
                    &[".config"],
                    application
                ),
                Some(PathBuf::from("/home/user/.config/skit")),
                "{blank:?}"
            );
        }
        assert_eq!(
            platform_directory(
                Some(OsString::from("  /redirect  ")),
                home,
                &[".config"],
                application
            ),
            Some(PathBuf::from("/redirect/skit"))
        );
        // A blank home directory names nothing either, so no relative path can escape.
        assert_eq!(
            platform_directory(None, Some(OsString::new()), &[".config"], application),
            None
        );
    }

    /// A value that is not Unicode still names a directory.
    ///
    /// Version 0.4 removes the spaces around a redirect value. Bytes that are not Unicode hold no
    /// spaces this rule can read, and they name a real directory on this host, so they stay whole.
    #[test]
    #[cfg(unix)]
    fn a_redirect_that_is_not_unicode_still_names_a_directory() {
        use std::os::unix::ffi::OsStringExt as _;

        let value = OsString::from_vec(b"/redirect/\xff".to_vec());

        assert_eq!(
            platform_directory(Some(value.clone()), None, &[], &[APPLICATION]),
            Some(PathBuf::from(value).join(APPLICATION))
        );
    }

    /// An override names a directory when it holds anything, and an empty one names none.
    ///
    /// Version 0.4 keeps the value whenever it is truthy (`skit-oracle/src/skit/paths.py:16-29`),
    /// so an empty variable leaves the host directory to answer, and spaces inside a value stay.
    #[test]
    fn an_override_directory_answers_unless_the_value_is_empty() {
        assert_eq!(
            override_directory(Some(OsString::from("/library"))),
            Some(PathBuf::from("/library"))
        );
        assert_eq!(
            override_directory(Some(OsString::from(" /spaced "))),
            Some(PathBuf::from(" /spaced "))
        );
        assert_eq!(override_directory(Some(OsString::new())), None);
        assert_eq!(override_directory(None), None);
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
                &[".local", "share"],
                &[APPLICATION]
            )
        );
        assert_eq!(
            platform_state_dir(),
            platform_directory(
                env::var_os("XDG_STATE_HOME"),
                home.clone(),
                &[".local", "state"],
                &[APPLICATION]
            )
        );
        assert_eq!(
            platform_config_dir(),
            platform_directory(
                env::var_os("XDG_CONFIG_HOME"),
                home,
                &[".config"],
                &[APPLICATION]
            )
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
            &[APPLICATION],
        );

        assert_eq!(platform_data_dir(), expected);
        assert_eq!(platform_state_dir(), expected);
        assert_eq!(platform_config_dir(), expected);
    }

    /// Windows keeps the state and the configuration with the data, below the local root, and it
    /// names the application twice there (`platformdirs/windows.py:29-33,36-44,59-61`).
    #[test]
    #[cfg(windows)]
    fn each_platform_directory_reads_the_variables_this_host_names() {
        let data = platform_directory(
            env::var_os("LOCALAPPDATA").or_else(|| env::var_os("APPDATA")),
            None,
            &[],
            &[APPLICATION, APPLICATION],
        );

        assert_eq!(platform_data_dir(), data);
        assert_eq!(platform_state_dir(), data);
        assert_eq!(platform_config_dir(), data);
    }
}
