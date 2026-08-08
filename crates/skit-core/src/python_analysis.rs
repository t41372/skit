use std::collections::BTreeSet;
use std::fs;
use std::path::Path;

use tree_sitter::{Node, Parser};

/// Suggest installable third-party distributions referenced by Python imports.
///
/// Parsing is lazy at this call boundary: listing and ordinary metadata paths never
/// instantiate tree-sitter. Any syntax error degrades to no suggestions, matching the
/// Python-era AST analyzer. Relative imports, standard-library names, private
/// underscore-led modules, and sibling local modules/packages are excluded.
#[must_use]
pub fn suggest_python_dependencies(text: &str, script_dir: Option<&Path>) -> Vec<String> {
    let mut parser = Parser::new();
    let language = tree_sitter_python::LANGUAGE.into();
    if parser.set_language(&language).is_err() {
        return Vec::new();
    }
    let Some(tree) = parser.parse(text, None) else {
        return Vec::new();
    };
    if tree.root_node().has_error() {
        return Vec::new();
    }

    let mut imports = BTreeSet::new();
    collect_imports(tree.root_node(), text, &mut imports);
    imports
        .into_iter()
        .filter(|module| {
            !module.starts_with('_')
                && !is_stdlib(module)
                && !is_local_module(script_dir, module)
        })
        .map(|module| distribution_for_import(&module).to_owned())
        .filter(|name| valid_distribution_name(name))
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn collect_imports(node: Node<'_>, source: &str, output: &mut BTreeSet<String>) {
    match node.kind() {
        "import_statement" => collect_plain_imports(node, source, output),
        "import_from_statement" => collect_from_import(node, source, output),
        _ => {}
    }

    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        collect_imports(child, source, output);
    }
}

fn collect_plain_imports(node: Node<'_>, source: &str, output: &mut BTreeSet<String>) {
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "dotted_name" => insert_top_level(child, source, output),
            "aliased_import" => {
                if let Some(name) = child.child_by_field_name("name") {
                    insert_top_level(name, source, output);
                }
            }
            _ => {}
        }
    }
}

fn collect_from_import(node: Node<'_>, source: &str, output: &mut BTreeSet<String>) {
    let Some(module) = node.child_by_field_name("module_name") else {
        return;
    };
    if module.kind() == "dotted_name" {
        insert_top_level(module, source, output);
    }
}

fn insert_top_level(node: Node<'_>, source: &str, output: &mut BTreeSet<String>) {
    let Some(text) = source.get(node.start_byte()..node.end_byte()) else {
        return;
    };
    if let Some(module) = text.split('.').next().filter(|module| !module.is_empty()) {
        output.insert(module.to_owned());
    }
}

fn is_local_module(script_dir: Option<&Path>, module: &str) -> bool {
    let Some(directory) = script_dir else {
        return false;
    };
    if directory.join(format!("{module}.py")).is_file() {
        return true;
    }
    let package = directory.join(module);
    let Ok(children) = fs::read_dir(package) else {
        return false;
    };
    children.filter_map(Result::ok).any(|entry| {
        entry.path().is_file()
            && entry
                .path()
                .extension()
                .is_some_and(|extension| extension.eq_ignore_ascii_case("py"))
    })
}

fn valid_distribution_name(name: &str) -> bool {
    let bytes = name.as_bytes();
    let Some(first) = bytes.first() else {
        return false;
    };
    let Some(last) = bytes.last() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && last.is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}

fn distribution_for_import(module: &str) -> &str {
    match module {
        "PIL" => "Pillow",
        "cv2" => "opencv-python",
        "yaml" => "PyYAML",
        "bs4" => "beautifulsoup4",
        "sklearn" => "scikit-learn",
        "skimage" => "scikit-image",
        "dotenv" => "python-dotenv",
        "dateutil" => "python-dateutil",
        "serial" => "pyserial",
        "jwt" => "PyJWT",
        "docx" => "python-docx",
        "pptx" => "python-pptx",
        "fitz" => "PyMuPDF",
        "OpenSSL" => "pyOpenSSL",
        "Crypto" => "pycryptodome",
        "Cryptodome" => "pycryptodomex",
        "git" => "GitPython",
        "attr" => "attrs",
        "slugify" => "python-slugify",
        "usb" => "pyusb",
        "win32com" | "win32api" => "pywin32",
        _ => module,
    }
}

fn is_stdlib(module: &str) -> bool {
    matches!(
        module,
        "abc"
            | "antigravity"
            | "argparse"
            | "array"
            | "ast"
            | "asyncio"
            | "atexit"
            | "base64"
            | "bdb"
            | "binascii"
            | "bisect"
            | "builtins"
            | "bz2"
            | "cProfile"
            | "calendar"
            | "cmath"
            | "cmd"
            | "code"
            | "codecs"
            | "codeop"
            | "collections"
            | "colorsys"
            | "compileall"
            | "concurrent"
            | "configparser"
            | "contextlib"
            | "contextvars"
            | "copy"
            | "copyreg"
            | "csv"
            | "ctypes"
            | "curses"
            | "dataclasses"
            | "datetime"
            | "dbm"
            | "decimal"
            | "difflib"
            | "dis"
            | "doctest"
            | "email"
            | "encodings"
            | "ensurepip"
            | "enum"
            | "errno"
            | "faulthandler"
            | "fcntl"
            | "filecmp"
            | "fileinput"
            | "fnmatch"
            | "fractions"
            | "ftplib"
            | "functools"
            | "gc"
            | "genericpath"
            | "getopt"
            | "getpass"
            | "gettext"
            | "glob"
            | "graphlib"
            | "grp"
            | "gzip"
            | "hashlib"
            | "heapq"
            | "hmac"
            | "html"
            | "http"
            | "idlelib"
            | "imaplib"
            | "importlib"
            | "inspect"
            | "io"
            | "ipaddress"
            | "itertools"
            | "json"
            | "keyword"
            | "linecache"
            | "locale"
            | "logging"
            | "lzma"
            | "mailbox"
            | "marshal"
            | "math"
            | "mimetypes"
            | "mmap"
            | "modulefinder"
            | "msvcrt"
            | "multiprocessing"
            | "netrc"
            | "nt"
            | "ntpath"
            | "nturl2path"
            | "numbers"
            | "opcode"
            | "operator"
            | "optparse"
            | "os"
            | "pathlib"
            | "pdb"
            | "pickle"
            | "pickletools"
            | "pkgutil"
            | "platform"
            | "plistlib"
            | "poplib"
            | "posix"
            | "posixpath"
            | "pprint"
            | "profile"
            | "pstats"
            | "pty"
            | "pwd"
            | "py_compile"
            | "pyclbr"
            | "pydoc"
            | "pydoc_data"
            | "pyexpat"
            | "queue"
            | "quopri"
            | "random"
            | "re"
            | "readline"
            | "reprlib"
            | "resource"
            | "rlcompleter"
            | "runpy"
            | "sched"
            | "secrets"
            | "select"
            | "selectors"
            | "shelve"
            | "shlex"
            | "shutil"
            | "signal"
            | "site"
            | "smtplib"
            | "socket"
            | "socketserver"
            | "sqlite3"
            | "sre_compile"
            | "sre_constants"
            | "sre_parse"
            | "ssl"
            | "stat"
            | "statistics"
            | "string"
            | "stringprep"
            | "struct"
            | "subprocess"
            | "symtable"
            | "sys"
            | "sysconfig"
            | "syslog"
            | "tabnanny"
            | "tarfile"
            | "tempfile"
            | "termios"
            | "textwrap"
            | "this"
            | "threading"
            | "time"
            | "timeit"
            | "tkinter"
            | "token"
            | "tokenize"
            | "tomllib"
            | "trace"
            | "traceback"
            | "tracemalloc"
            | "tty"
            | "turtle"
            | "turtledemo"
            | "types"
            | "typing"
            | "unicodedata"
            | "unittest"
            | "urllib"
            | "uuid"
            | "venv"
            | "warnings"
            | "wave"
            | "weakref"
            | "webbrowser"
            | "winreg"
            | "winsound"
            | "wsgiref"
            | "xml"
            | "xmlrpc"
            | "zipapp"
            | "zipfile"
            | "zipimport"
            | "zlib"
            | "zoneinfo"
    )
}
