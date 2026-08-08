use std::fs;
use std::io::Read;
use std::path::Path;

/// The executable-file rule for one operating-system family.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExecutablePolicy<'a> {
    Posix,
    Windows(&'a str),
}

/// A language family. It controls source-file and template behavior.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Family {
    Interpreted,
    Binary,
    Template,
}

/// The package dependency mechanism for a language.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DepsFlavor {
    None,
    Uv,
    Npm,
}

/// Stable, parser-free traits for one registered kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageSpec {
    pub kind: &'static str,
    pub family: Family,
    pub glyph: &'static str,
    pub extensions: &'static [&'static str],
    pub shebangs: &'static [&'static str],
    pub default_interpreter: &'static str,
    pub stored_name: &'static str,
    pub supports_modes: bool,
    pub deps_flavor: DepsFlavor,
    pub takes_argv: bool,
    pub placeholder_params: bool,
}

const PYTHON_EXTENSIONS: &[&str] = &[".py"];
const PYTHON_SHEBANGS: &[&str] = &["python", "python3"];
const SHELL_EXTENSIONS: &[&str] = &[".sh", ".bash", ".zsh"];
const SHELL_SHEBANGS: &[&str] = &["bash", "sh", "zsh", "dash", "ash", "ksh"];
const FISH_EXTENSIONS: &[&str] = &[".fish"];
const FISH_SHEBANGS: &[&str] = &["fish"];
const JS_EXTENSIONS: &[&str] = &[".js", ".mjs", ".cjs"];
const JS_SHEBANGS: &[&str] = &["node", "deno", "bun"];
const TS_EXTENSIONS: &[&str] = &[".ts", ".mts", ".cts"];
const POWERSHELL_EXTENSIONS: &[&str] = &[".ps1"];
const POWERSHELL_SHEBANGS: &[&str] = &["pwsh", "powershell"];
const RUBY_EXTENSIONS: &[&str] = &[".rb"];
const RUBY_SHEBANGS: &[&str] = &["ruby"];
const PERL_EXTENSIONS: &[&str] = &[".pl"];
const PERL_SHEBANGS: &[&str] = &["perl"];
const LUA_EXTENSIONS: &[&str] = &[".lua"];
const LUA_SHEBANGS: &[&str] = &["lua", "luajit"];
const R_EXTENSIONS: &[&str] = &[".r"];
const R_SHEBANGS: &[&str] = &["Rscript"];
const PROMPT_EXTENSIONS: &[&str] = &[".prompt.md", ".prompt"];
const EMPTY: &[&str] = &[];

const KNOWN_KINDS: &[&str] = &[
    "python",
    "shell",
    "fish",
    "js",
    "ts",
    "powershell",
    "ruby",
    "perl",
    "lua",
    "r",
    "exe",
    "command",
    "prompt",
];

const PYTHON: LanguageSpec = interpreted(
    "python",
    "⬡",
    PYTHON_EXTENSIONS,
    PYTHON_SHEBANGS,
    "",
    "script.py",
    DepsFlavor::Uv,
);
const SHELL: LanguageSpec = interpreted(
    "shell",
    "#",
    SHELL_EXTENSIONS,
    SHELL_SHEBANGS,
    "bash",
    "script.sh",
    DepsFlavor::None,
);
const FISH: LanguageSpec = interpreted(
    "fish",
    "∿",
    FISH_EXTENSIONS,
    FISH_SHEBANGS,
    "fish",
    "script.fish",
    DepsFlavor::None,
);
const JS: LanguageSpec = interpreted(
    "js",
    "✦",
    JS_EXTENSIONS,
    JS_SHEBANGS,
    "",
    "script.js",
    DepsFlavor::Npm,
);
const TS: LanguageSpec = interpreted(
    "ts",
    "✧",
    TS_EXTENSIONS,
    EMPTY,
    "",
    "script.ts",
    DepsFlavor::Npm,
);
const POWERSHELL: LanguageSpec = interpreted(
    "powershell",
    "»",
    POWERSHELL_EXTENSIONS,
    POWERSHELL_SHEBANGS,
    "pwsh",
    "script.ps1",
    DepsFlavor::None,
);
const RUBY: LanguageSpec = interpreted(
    "ruby",
    "◆",
    RUBY_EXTENSIONS,
    RUBY_SHEBANGS,
    "ruby",
    "script.rb",
    DepsFlavor::None,
);
const PERL: LanguageSpec = interpreted(
    "perl",
    "◈",
    PERL_EXTENSIONS,
    PERL_SHEBANGS,
    "perl",
    "script.pl",
    DepsFlavor::None,
);
const LUA: LanguageSpec = interpreted(
    "lua",
    "○",
    LUA_EXTENSIONS,
    LUA_SHEBANGS,
    "lua",
    "script.lua",
    DepsFlavor::None,
);
const R: LanguageSpec = interpreted(
    "r",
    "◇",
    R_EXTENSIONS,
    R_SHEBANGS,
    "Rscript",
    "script.r",
    DepsFlavor::None,
);
const EXE: LanguageSpec = LanguageSpec {
    kind: "exe",
    family: Family::Binary,
    glyph: "▶",
    extensions: EMPTY,
    shebangs: EMPTY,
    default_interpreter: "",
    stored_name: "",
    supports_modes: false,
    deps_flavor: DepsFlavor::None,
    takes_argv: true,
    placeholder_params: false,
};
const COMMAND: LanguageSpec = LanguageSpec {
    kind: "command",
    family: Family::Template,
    glyph: "$",
    extensions: EMPTY,
    shebangs: EMPTY,
    default_interpreter: "",
    stored_name: "",
    supports_modes: false,
    deps_flavor: DepsFlavor::None,
    takes_argv: false,
    placeholder_params: true,
};
const PROMPT: LanguageSpec = LanguageSpec {
    kind: "prompt",
    family: Family::Interpreted,
    glyph: "✎",
    extensions: PROMPT_EXTENSIONS,
    shebangs: EMPTY,
    default_interpreter: "",
    stored_name: "prompt.md",
    supports_modes: true,
    deps_flavor: DepsFlavor::None,
    takes_argv: false,
    placeholder_params: true,
};

const fn interpreted(
    kind: &'static str,
    glyph: &'static str,
    extensions: &'static [&'static str],
    shebangs: &'static [&'static str],
    default_interpreter: &'static str,
    stored_name: &'static str,
    deps_flavor: DepsFlavor,
) -> LanguageSpec {
    LanguageSpec {
        kind,
        family: Family::Interpreted,
        glyph,
        extensions,
        shebangs,
        default_interpreter,
        stored_name,
        supports_modes: true,
        deps_flavor,
        takes_argv: true,
        placeholder_params: false,
    }
}

/// Return the stable kind order from the current Python registry.
#[must_use]
pub const fn known_kinds() -> &'static [&'static str] {
    KNOWN_KINDS
}

/// Resolve one kind without loading parser-backed capabilities.
#[must_use]
pub const fn spec_for(kind: &str) -> Option<&'static LanguageSpec> {
    match kind.as_bytes() {
        b"python" => Some(&PYTHON),
        b"shell" => Some(&SHELL),
        b"fish" => Some(&FISH),
        b"js" => Some(&JS),
        b"ts" => Some(&TS),
        b"powershell" => Some(&POWERSHELL),
        b"ruby" => Some(&RUBY),
        b"perl" => Some(&PERL),
        b"lua" => Some(&LUA),
        b"r" => Some(&R),
        b"exe" => Some(&EXE),
        b"command" => Some(&COMMAND),
        b"prompt" => Some(&PROMPT),
        _ => None,
    }
}

/// Return the historical in-store filename. Unknown kinds use `payload` for forward
/// compatibility with newer metadata.
#[must_use]
pub fn stored_name(kind: &str) -> &'static str {
    spec_for(kind).map_or("payload", |spec| spec.stored_name)
}

/// Infer a registered kind from a filename only.
#[must_use]
pub fn kind_for_extension(filename: &str) -> Option<&'static str> {
    let lowered = filename.to_ascii_lowercase();
    for kind in KNOWN_KINDS {
        let Some(spec) = spec_for(kind) else {
            continue;
        };
        if spec
            .extensions
            .iter()
            .any(|extension| lowered.ends_with(extension))
        {
            return Some(spec.kind);
        }
    }
    None
}

/// Infer a kind from a path with an explicit executable-file policy.
///
/// Registered extensions win over shebangs. A registered shebang wins over the
/// executable fallback. The explicit `force_exe` flag wins over all inference.
#[must_use]
pub fn infer_kind_with_policy(
    path: &Path,
    force_exe: bool,
    policy: ExecutablePolicy<'_>,
) -> &'static str {
    if force_exe {
        return "exe";
    }
    if let Some(filename) = path.file_name().and_then(|name| name.to_str())
        && let Some(kind) = kind_for_extension(filename)
    {
        return kind;
    }
    if path.is_file() {
        if let Some(kind) = shebang_kind(path) {
            return kind;
        }
        if is_executable_file(path, policy) {
            return "exe";
        }
    }
    "unknown"
}

/// Infer a kind with the current platform's executable-file rule.
#[must_use]
pub fn infer_kind(path: &Path, force_exe: bool) -> &'static str {
    #[cfg(windows)]
    {
        let pathext = std::env::var("PATHEXT").unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_owned());
        return infer_kind_with_policy(path, force_exe, ExecutablePolicy::Windows(&pathext));
    }
    #[cfg(unix)]
    {
        infer_kind_with_policy(path, force_exe, ExecutablePolicy::Posix)
    }
    #[cfg(not(any(unix, windows)))]
    {
        infer_kind_with_policy(path, force_exe, ExecutablePolicy::Windows(""))
    }
}

fn shebang_kind(path: &Path) -> Option<&'static str> {
    let file = fs::File::open(path).ok()?;
    let mut bytes = Vec::with_capacity(512);
    file.take(512).read_to_end(&mut bytes).ok()?;
    let end = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .unwrap_or(bytes.len());
    let line = String::from_utf8_lossy(&bytes[..end]);
    let program = shebang_program_from_line(&line)?;
    kind_for_program(program)
}

fn is_executable_file(path: &Path, policy: ExecutablePolicy<'_>) -> bool {
    if !path.is_file() {
        return false;
    }
    match policy {
        ExecutablePolicy::Windows(pathext) => {
            let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
                return false;
            };
            let pathext = if pathext.is_empty() {
                ".COM;.EXE;.BAT;.CMD"
            } else {
                pathext
            };
            pathext
                .split(';')
                .filter(|item| !item.is_empty())
                .any(|item| item.trim_start_matches('.').eq_ignore_ascii_case(extension))
        }
        ExecutablePolicy::Posix => posix_executable(path),
    }
}

#[cfg(unix)]
fn posix_executable(path: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;

    fs::metadata(path).is_ok_and(|metadata| metadata.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn posix_executable(_path: &Path) -> bool {
    false
}

/// Return the executable basename from one shebang line.
#[must_use]
pub fn shebang_program_from_line(line: &str) -> Option<&str> {
    let payload = line.strip_prefix("#!")?;
    let mut tokens = payload.split_whitespace();
    let first = tokens.next()?;
    let mut program = basename(first);
    if program == "env" {
        program = tokens.find(|token| !token.starts_with('-')).map(basename)?;
    }
    Some(program)
}

/// Return the kind named by a text blob's first-line shebang.
#[must_use]
pub fn kind_for_shebang_text(text: &str) -> Option<&'static str> {
    let line = text.split_once('\n').map_or(text, |(first, _)| first);
    let program = shebang_program_from_line(line)?;
    kind_for_program(program)
}

/// Return the `requires-python` constraint implied by a versioned Python 3 shebang.
#[must_use]
pub fn python_version_pin(program: Option<&str>) -> String {
    let Some(version) = program.and_then(|program| program.strip_prefix("python3.")) else {
        return String::new();
    };
    let mut parts = version.split('.');
    let Some(minor_text) = parts.next() else {
        return String::new();
    };
    let Ok(minor) = minor_text.parse::<u32>() else {
        return String::new();
    };
    if parts
        .clone()
        .any(|part| part.is_empty() || !part.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return String::new();
    }
    format!(">=3.{version},<3.{}", minor + 1)
}

fn kind_for_program(program: &str) -> Option<&'static str> {
    for kind in KNOWN_KINDS {
        let spec = spec_for(kind)?;
        if spec.shebangs.contains(&program) {
            return Some(spec.kind);
        }
    }
    is_versioned_python(program).then_some("python")
}

fn is_versioned_python(program: &str) -> bool {
    if program == "python" || program == "python3" {
        return true;
    }
    let Some(version) = program.strip_prefix("python3.") else {
        return false;
    };
    !version.is_empty()
        && version
            .split('.')
            .all(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}
