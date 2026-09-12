//! Pure random and stable sandbox identity and ownership contracts.

use std::{
    collections::{BTreeMap, BTreeSet},
    error::Error,
    fmt,
    path::PathBuf,
    str::FromStr,
};

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    de::{self, DeserializeOwned},
};
use serde_json::Value;

use crate::{ArtifactError, canonical_json_bytes, require};

/// Stable namespace literal for real walker corpus sandboxes.
pub const STABLE_SANDBOX_NAMESPACE: &str = "skit-ui-walker-v1";
/// Stable namespace marker file name.
pub const NAMESPACE_MARKER_FILE: &str = ".skit-ui-walker-namespace.json";
/// Stable sandbox marker file name.
pub const SANDBOX_MARKER_FILE: &str = ".skit-ui-walker-sandbox.json";
/// Stable namespace lease directory name.
pub const LEASE_DIRECTORY: &str = "leases";
/// Stable namespace sandbox directory name.
pub const SANDBOX_DIRECTORY: &str = "sandboxes";

const OWNERSHIP_SCHEMA: u16 = 1;

/// A rejected stable review-profile identifier.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SafeProfileIdError(&'static str);

impl fmt::Display for SafeProfileIdError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

impl Error for SafeProfileIdError {}

/// A validated stable review-profile identifier.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SafeProfileId(String);

impl SafeProfileId {
    /// Return the canonical identifier text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SafeProfileId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for SafeProfileId {
    type Err = SafeProfileIdError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        Self::try_from(value)
    }
}

impl TryFrom<&str> for SafeProfileId {
    type Error = SafeProfileIdError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        validate_profile_id(value)?;
        Ok(Self(value.to_owned()))
    }
}

impl TryFrom<String> for SafeProfileId {
    type Error = SafeProfileIdError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        validate_profile_id(&value)?;
        Ok(Self(value))
    }
}

impl Serialize for SafeProfileId {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for SafeProfileId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

fn validate_profile_id(value: &str) -> Result<(), SafeProfileIdError> {
    if value.is_empty() || value.len() > 64 {
        return Err(SafeProfileIdError(
            "profile id must contain 1 through 64 ASCII bytes",
        ));
    }
    let bytes = value.as_bytes();
    let endpoint = |byte: u8| byte.is_ascii_lowercase() || byte.is_ascii_digit();
    if !endpoint(bytes[0])
        || !endpoint(bytes[bytes.len() - 1])
        || !bytes.iter().all(|byte| endpoint(*byte) || *byte == b'-')
    {
        return Err(SafeProfileIdError(
            "profile id must use lowercase ASCII letters, digits, and interior hyphens",
        ));
    }
    if is_reserved_profile_id(value) {
        return Err(SafeProfileIdError("profile id is a reserved device name"));
    }
    Ok(())
}

fn is_reserved_profile_id(value: &str) -> bool {
    matches!(value, "con" | "prn" | "aux" | "nul")
        || value.as_bytes().get(0..3).is_some_and(|prefix| {
            matches!(prefix, b"com" | b"lpt")
                && value.len() == 4
                && matches!(value.as_bytes()[3], b'1'..=b'9')
        })
}

/// Operating-system family declared by random or stable sandbox evidence.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxPlatform {
    /// Linux stable namespace semantics.
    Linux,
    /// macOS random-sandbox semantics.
    Macos,
    /// Windows stable namespace semantics.
    Windows,
}

/// Sandbox allocation mode declared by run metadata.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum SandboxMode {
    /// Existing isolated temporary-directory behavior.
    Random,
    /// Stable leased namespace behavior for final corpus generation.
    Stable,
}

/// Closed stable ownership-marker kinds.
#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum OwnershipMarkerKind {
    /// Persistent parent initialization lock.
    ParentInitLock,
    /// Initialized namespace root.
    Namespace,
    /// Per-profile lease file.
    ProfileLease,
    /// Per-profile sandbox root.
    Sandbox,
}

/// Exact roots owned by one stable profile sandbox.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxRoots {
    root: String,
    data: String,
    state: String,
    config: String,
    home: String,
    cwd: String,
    external: String,
    system_temp: String,
}

impl SandboxRoots {
    /// Derive every exact child root from one declared sandbox root.
    pub fn new(platform: SandboxPlatform, root: impl Into<String>) -> Result<Self, ArtifactError> {
        Self::derive(platform, root.into())
    }

    /// Return the complete sandbox root.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Return the product data root.
    #[must_use]
    pub fn data(&self) -> &str {
        &self.data
    }

    /// Return the product state root.
    #[must_use]
    pub fn state(&self) -> &str {
        &self.state
    }

    /// Return the product configuration root.
    #[must_use]
    pub fn config(&self) -> &str {
        &self.config
    }

    /// Return the declared home root.
    #[must_use]
    pub fn home(&self) -> &str {
        &self.home
    }

    /// Return the declared working directory root.
    #[must_use]
    pub fn cwd(&self) -> &str {
        &self.cwd
    }

    /// Return the external-fixture root.
    #[must_use]
    pub fn external(&self) -> &str {
        &self.external
    }

    /// Return the profile-local system-temp root.
    #[must_use]
    pub fn system_temp(&self) -> &str {
        &self.system_temp
    }

    fn validate(&self, platform: SandboxPlatform) -> Result<(), ArtifactError> {
        let expected = Self::derive(platform, self.root.clone())?;
        require(
            self == &expected,
            "sandbox roots do not match the declared root",
        )
    }

    fn derive(platform: SandboxPlatform, root: String) -> Result<Self, ArtifactError> {
        validate_literal_path(platform, &root)?;
        Ok(Self {
            data: join_literal_path(platform, &root, "data")?,
            state: join_literal_path(platform, &root, "state")?,
            config: join_literal_path(platform, &root, "config")?,
            home: join_literal_path(platform, &root, "home")?,
            cwd: join_literal_path(platform, &root, "cwd")?,
            external: join_literal_path(platform, &root, "external")?,
            system_temp: join_literal_path(platform, &root, "system-temp")?,
            root,
        })
    }
}

/// Canonical sandbox metadata bound into one bundle run.
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct SandboxMetadata {
    schema: u16,
    mode: SandboxMode,
    namespace: String,
    root: String,
    platform: SandboxPlatform,
    profiles: BTreeMap<SafeProfileId, String>,
}

impl SandboxMetadata {
    /// Construct metadata for one existing random sandbox.
    pub fn random(
        platform: SandboxPlatform,
        root: impl Into<String>,
        profile: SafeProfileId,
    ) -> Result<Self, ArtifactError> {
        let root = root.into();
        Self::from_parts(
            SandboxMode::Random,
            platform,
            root.clone(),
            BTreeMap::from([(profile, root)]),
        )
    }

    /// Construct metadata for one stable namespace and its review profiles.
    pub fn stable(
        platform: SandboxPlatform,
        root: impl Into<String>,
        profiles: BTreeSet<SafeProfileId>,
    ) -> Result<Self, ArtifactError> {
        let root = root.into();
        let profile_roots = profiles
            .into_iter()
            .map(|profile| {
                let layout = ProfileSandboxLayout::derive(platform, &root, &profile)?;
                Ok((profile, layout.sandbox_root))
            })
            .collect::<Result<_, ArtifactError>>()?;
        Self::from_parts(SandboxMode::Stable, platform, root, profile_roots)
    }

    fn from_parts(
        mode: SandboxMode,
        platform: SandboxPlatform,
        root: String,
        profiles: BTreeMap<SafeProfileId, String>,
    ) -> Result<Self, ArtifactError> {
        let metadata = Self {
            schema: OWNERSHIP_SCHEMA,
            mode,
            namespace: STABLE_SANDBOX_NAMESPACE.to_owned(),
            root,
            platform,
            profiles,
        };
        metadata.validate()?;
        Ok(metadata)
    }

    /// Return the declared sandbox mode.
    #[must_use]
    pub const fn mode(&self) -> SandboxMode {
        self.mode
    }

    /// Return the walker namespace literal.
    #[must_use]
    pub fn namespace(&self) -> &str {
        &self.namespace
    }

    /// Return the declared sandbox or namespace root.
    #[must_use]
    pub fn root(&self) -> &str {
        &self.root
    }

    /// Return the declared platform.
    #[must_use]
    pub const fn platform(&self) -> SandboxPlatform {
        self.platform
    }

    /// Return the sorted review-profile to literal-root map.
    #[must_use]
    pub fn profiles(&self) -> &BTreeMap<SafeProfileId, String> {
        &self.profiles
    }

    /// Encode exact canonical metadata bytes without a trailing newline.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        canonical_contract_bytes(self)
    }

    /// Decode canonical bytes and require the exact declared metadata.
    pub fn decode_and_validate(bytes: &[u8], expected: &Self) -> Result<Self, ArtifactError> {
        decode_contract(bytes, expected)
    }
}

impl ValidatedContract for SandboxMetadata {
    fn validate(&self) -> Result<(), ArtifactError> {
        require(
            self.schema == OWNERSHIP_SCHEMA,
            "sandbox metadata schema is invalid",
        )?;
        require(
            self.namespace == STABLE_SANDBOX_NAMESPACE,
            "sandbox metadata namespace is invalid",
        )?;
        validate_literal_path(self.platform, &self.root)?;
        require(
            !self.profiles.is_empty(),
            "sandbox metadata has no profiles",
        )?;
        match self.mode {
            SandboxMode::Random => {
                require(
                    self.profiles.len() == 1,
                    "random sandbox metadata must declare one profile",
                )?;
                require(
                    self.profiles.values().all(|root| root == &self.root),
                    "random sandbox profile root does not match the sandbox root",
                )
            }
            SandboxMode::Stable => {
                for (profile, root) in &self.profiles {
                    let expected =
                        ProfileSandboxLayout::derive(self.platform, &self.root, profile)?;
                    require(
                        root == &expected.sandbox_root,
                        "stable sandbox profile root does not match the namespace layout",
                    )?;
                }
                Ok(())
            }
        }
    }
}

/// Validate one typed sandbox metadata object.
pub fn validate_sandbox_metadata(metadata: &SandboxMetadata) -> Result<(), ArtifactError> {
    metadata.validate()
}

fn validate_literal_path(platform: SandboxPlatform, value: &str) -> Result<(), ArtifactError> {
    require(
        !value.is_empty()
            && !value
                .bytes()
                .any(|byte| matches!(byte, b'\0' | b'\n' | b'\r')),
        "sandbox path is empty or contains a control byte",
    )?;
    match platform {
        SandboxPlatform::Linux | SandboxPlatform::Macos => validate_unix_literal_path(value),
        SandboxPlatform::Windows => validate_windows_literal_path(value),
    }
}

fn validate_unix_literal_path(value: &str) -> Result<(), ArtifactError> {
    require(
        value.starts_with('/'),
        "sandbox path is not an absolute platform path",
    )?;
    require(
        value == "/"
            || value
                .split('/')
                .skip(1)
                .all(|component| !component.is_empty() && component != "." && component != ".."),
        "sandbox path has a noncanonical component",
    )
}

fn validate_windows_literal_path(value: &str) -> Result<(), ArtifactError> {
    require(
        !value.contains('/'),
        "sandbox Windows path uses a mixed separator",
    )?;
    let bytes = value.as_bytes();
    let components = if let Some(unc_path) = value.strip_prefix("\\\\") {
        let components = unc_path.split('\\').collect::<Vec<_>>();
        require(
            components.len() >= 2,
            "sandbox path is not an absolute platform path",
        )?;
        components
    } else {
        require(
            bytes.len() >= 3
                && bytes[0].is_ascii_uppercase()
                && bytes[1] == b':'
                && bytes[2] == b'\\',
            "sandbox path is not an absolute platform path",
        )?;
        if bytes.len() == 3 {
            return Ok(());
        }
        value[3..].split('\\').collect::<Vec<_>>()
    };
    for component in components {
        validate_windows_component(component)?;
    }
    Ok(())
}

fn validate_windows_component(component: &str) -> Result<(), ArtifactError> {
    require(
        !component.is_empty() && component != "." && component != "..",
        "sandbox path has a noncanonical component",
    )?;
    require(
        !component.ends_with('.') && !component.ends_with(' '),
        "sandbox Windows path component has a trailing dot or space",
    )?;
    require(
        !component.bytes().any(|byte| {
            byte <= 31 || matches!(byte, b'<' | b'>' | b'"' | b'|' | b'?' | b'*' | b':')
        }),
        "sandbox Windows path component contains a reserved character",
    )?;
    let basename = component
        .split_once('.')
        .map_or(component, |(basename, _)| basename)
        .trim_end_matches(' ')
        .to_ascii_lowercase();
    require(
        !is_reserved_windows_device_basename(&basename),
        "sandbox Windows path component uses a reserved device name",
    )
}

fn is_reserved_windows_device_basename(basename: &str) -> bool {
    if is_reserved_profile_id(basename) {
        return true;
    }
    basename
        .strip_prefix("com")
        .or_else(|| basename.strip_prefix("lpt"))
        .is_some_and(|suffix| matches!(suffix, "¹" | "²" | "³"))
}

fn join_literal_path(
    platform: SandboxPlatform,
    parent: &str,
    component: &str,
) -> Result<String, ArtifactError> {
    validate_literal_path(platform, parent)?;
    let separator = match platform {
        SandboxPlatform::Linux | SandboxPlatform::Macos => '/',
        SandboxPlatform::Windows => '\\',
    };
    if parent.ends_with(separator) {
        Ok(format!("{parent}{component}"))
    } else {
        Ok(format!("{parent}{separator}{component}"))
    }
}

fn split_literal_parent(
    platform: SandboxPlatform,
    path: &str,
) -> Result<(&str, &str), ArtifactError> {
    validate_literal_path(platform, path)?;
    let separator = match platform {
        SandboxPlatform::Linux | SandboxPlatform::Macos => '/',
        SandboxPlatform::Windows => '\\',
    };
    let index = path
        .rfind(separator)
        .ok_or_else(|| ArtifactError::new("sandbox path has no parent"))?;
    let component = &path[index + separator.len_utf8()..];
    let parent = if index == 0 {
        &path[..separator.len_utf8()]
    } else if platform == SandboxPlatform::Windows && index == 2 {
        &path[..index + separator.len_utf8()]
    } else {
        &path[..index]
    };
    validate_literal_path(platform, parent)?;
    Ok((parent, component))
}

fn validate_stable_platform(platform: SandboxPlatform) -> Result<(), ArtifactError> {
    require(
        platform != SandboxPlatform::Macos,
        "stable sandbox platform is not supported",
    )
}

fn validate_namespace_root(
    platform: SandboxPlatform,
    namespace_root: &str,
) -> Result<(), ArtifactError> {
    validate_stable_platform(platform)?;
    let (_, component) = split_literal_parent(platform, namespace_root)?;
    require(
        component == STABLE_SANDBOX_NAMESPACE,
        "sandbox namespace root has the wrong name",
    )
}

/// Validate one canonical stable namespace-root literal for the declared platform.
pub fn validate_stable_namespace_root(
    platform: SandboxPlatform,
    namespace_root: &str,
) -> Result<(), ArtifactError> {
    validate_namespace_root(platform, namespace_root)
}

fn parent_init_lock_path(
    platform: SandboxPlatform,
    namespace_root: &str,
) -> Result<String, ArtifactError> {
    validate_namespace_root(platform, namespace_root)?;
    let (parent, _) = split_literal_parent(platform, namespace_root)?;
    join_literal_path(
        platform,
        parent,
        &format!(".{STABLE_SANDBOX_NAMESPACE}.init.lock"),
    )
}

struct ProfileSandboxLayout {
    lease_path: String,
    sandbox_root: String,
    cleanup_root: String,
}

impl ProfileSandboxLayout {
    fn derive(
        platform: SandboxPlatform,
        namespace_root: &str,
        profile: &SafeProfileId,
    ) -> Result<Self, ArtifactError> {
        validate_namespace_root(platform, namespace_root)?;
        let separator = match platform {
            SandboxPlatform::Linux | SandboxPlatform::Macos => '/',
            SandboxPlatform::Windows => '\\',
        };
        let leases_root = format!("{namespace_root}{separator}{LEASE_DIRECTORY}");
        let sandboxes_root = format!("{namespace_root}{separator}{SANDBOX_DIRECTORY}");
        Ok(Self {
            lease_path: format!("{leases_root}{separator}{profile}.lock"),
            sandbox_root: format!("{sandboxes_root}{separator}{profile}"),
            cleanup_root: format!("{sandboxes_root}{separator}.cleanup-{profile}"),
        })
    }
}

fn validate_profile_sandbox_root(
    platform: SandboxPlatform,
    sandbox_root: &str,
    profile: &SafeProfileId,
) -> Result<(), ArtifactError> {
    let (sandboxes_root, profile_component) = split_literal_parent(platform, sandbox_root)?;
    require(
        profile_component == profile.as_str(),
        "sandbox root does not match the review profile",
    )?;
    let (namespace_root, directory) = split_literal_parent(platform, sandboxes_root)?;
    require(
        directory == SANDBOX_DIRECTORY,
        "sandbox root is outside the sandbox directory",
    )?;
    validate_namespace_root(platform, namespace_root)
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Ownership evidence for the persistent parent initialization lock.
pub struct ParentInitLockMarker {
    schema: u16,
    kind: OwnershipMarkerKind,
    namespace: String,
    platform: SandboxPlatform,
    namespace_root: String,
    init_lock: String,
}

impl ParentInitLockMarker {
    /// Construct exact parent-lock ownership evidence.
    pub fn new(
        platform: SandboxPlatform,
        namespace_root: impl Into<String>,
    ) -> Result<Self, ArtifactError> {
        let namespace_root = namespace_root.into();
        let init_lock = parent_init_lock_path(platform, &namespace_root)?;
        let marker = Self {
            schema: OWNERSHIP_SCHEMA,
            kind: OwnershipMarkerKind::ParentInitLock,
            namespace: STABLE_SANDBOX_NAMESPACE.to_owned(),
            platform,
            namespace_root,
            init_lock,
        };
        marker.validate()?;
        Ok(marker)
    }

    /// Encode exact canonical marker bytes without a trailing newline.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        canonical_contract_bytes(self)
    }

    /// Decode canonical bytes and require the exact declared ownership.
    pub fn decode_and_validate(bytes: &[u8], expected: &Self) -> Result<Self, ArtifactError> {
        decode_contract(bytes, expected)
    }
}

impl ValidatedContract for ParentInitLockMarker {
    fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(
            self.schema,
            self.kind,
            OwnershipMarkerKind::ParentInitLock,
            &self.namespace,
        )?;
        validate_namespace_root(self.platform, &self.namespace_root)?;
        let expected = parent_init_lock_path(self.platform, &self.namespace_root)?;
        require(
            self.init_lock == expected,
            "parent initialization lock path does not match the namespace root",
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Ownership evidence for an initialized stable namespace.
pub struct NamespaceMarker {
    schema: u16,
    kind: OwnershipMarkerKind,
    namespace: String,
    platform: SandboxPlatform,
    root: String,
    leases_root: String,
    sandboxes_root: String,
}

impl NamespaceMarker {
    /// Construct exact namespace ownership evidence.
    pub fn new(platform: SandboxPlatform, root: impl Into<String>) -> Result<Self, ArtifactError> {
        let root = root.into();
        validate_namespace_root(platform, &root)?;
        let marker = Self {
            schema: OWNERSHIP_SCHEMA,
            kind: OwnershipMarkerKind::Namespace,
            namespace: STABLE_SANDBOX_NAMESPACE.to_owned(),
            platform,
            leases_root: join_literal_path(platform, &root, LEASE_DIRECTORY)?,
            sandboxes_root: join_literal_path(platform, &root, SANDBOX_DIRECTORY)?,
            root,
        };
        marker.validate()?;
        Ok(marker)
    }

    /// Encode exact canonical marker bytes without a trailing newline.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        canonical_contract_bytes(self)
    }

    /// Decode canonical bytes and require the exact declared ownership.
    pub fn decode_and_validate(bytes: &[u8], expected: &Self) -> Result<Self, ArtifactError> {
        decode_contract(bytes, expected)
    }
}

impl ValidatedContract for NamespaceMarker {
    fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(
            self.schema,
            self.kind,
            OwnershipMarkerKind::Namespace,
            &self.namespace,
        )?;
        validate_namespace_root(self.platform, &self.root)?;
        require(
            self.leases_root == join_literal_path(self.platform, &self.root, LEASE_DIRECTORY)?
                && self.sandboxes_root
                    == join_literal_path(self.platform, &self.root, SANDBOX_DIRECTORY)?,
            "namespace roots do not match the namespace layout",
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Ownership evidence for one stable profile lease.
pub struct ProfileLeaseMarker {
    schema: u16,
    kind: OwnershipMarkerKind,
    namespace: String,
    platform: SandboxPlatform,
    profile: SafeProfileId,
    namespace_root: String,
    lease_path: String,
    sandbox_root: String,
    cleanup_root: String,
}

impl ProfileLeaseMarker {
    /// Construct exact profile-lease ownership evidence.
    pub fn new(
        platform: SandboxPlatform,
        profile: SafeProfileId,
        namespace_root: impl Into<String>,
    ) -> Result<Self, ArtifactError> {
        let namespace_root = namespace_root.into();
        let layout = ProfileSandboxLayout::derive(platform, &namespace_root, &profile)?;
        let marker = Self {
            schema: OWNERSHIP_SCHEMA,
            kind: OwnershipMarkerKind::ProfileLease,
            namespace: STABLE_SANDBOX_NAMESPACE.to_owned(),
            platform,
            profile,
            lease_path: layout.lease_path,
            sandbox_root: layout.sandbox_root,
            cleanup_root: layout.cleanup_root,
            namespace_root,
        };
        marker.validate()?;
        Ok(marker)
    }

    /// Return the owned review profile.
    #[must_use]
    pub fn profile(&self) -> &SafeProfileId {
        &self.profile
    }

    /// Encode exact canonical marker bytes without a trailing newline.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        canonical_contract_bytes(self)
    }

    /// Decode canonical bytes and require the exact declared ownership.
    pub fn decode_and_validate(bytes: &[u8], expected: &Self) -> Result<Self, ArtifactError> {
        decode_contract(bytes, expected)
    }
}

impl ValidatedContract for ProfileLeaseMarker {
    fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(
            self.schema,
            self.kind,
            OwnershipMarkerKind::ProfileLease,
            &self.namespace,
        )?;
        let expected =
            ProfileSandboxLayout::derive(self.platform, &self.namespace_root, &self.profile)?;
        require(
            self.lease_path == expected.lease_path
                && self.sandbox_root == expected.sandbox_root
                && self.cleanup_root == expected.cleanup_root,
            "profile lease paths do not match the namespace layout",
        )
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Ownership evidence for one stable profile sandbox.
pub struct SandboxMarker {
    schema: u16,
    kind: OwnershipMarkerKind,
    namespace: String,
    platform: SandboxPlatform,
    profile: SafeProfileId,
    roots: SandboxRoots,
}

impl SandboxMarker {
    /// Construct exact profile-sandbox ownership evidence.
    pub fn new(
        platform: SandboxPlatform,
        profile: SafeProfileId,
        namespace_root: impl Into<String>,
    ) -> Result<Self, ArtifactError> {
        let namespace_root = namespace_root.into();
        let layout = ProfileSandboxLayout::derive(platform, &namespace_root, &profile)?;
        let marker = Self {
            schema: OWNERSHIP_SCHEMA,
            kind: OwnershipMarkerKind::Sandbox,
            namespace: STABLE_SANDBOX_NAMESPACE.to_owned(),
            platform,
            profile,
            roots: SandboxRoots::new(platform, layout.sandbox_root)?,
        };
        marker.validate()?;
        Ok(marker)
    }

    /// Return the owned review profile.
    #[must_use]
    pub fn profile(&self) -> &SafeProfileId {
        &self.profile
    }

    /// Return every exact declared sandbox root.
    #[must_use]
    pub fn roots(&self) -> &SandboxRoots {
        &self.roots
    }

    /// Validate the marker's closed schema, kind, namespace, and roots.
    pub fn validate(&self) -> Result<(), ArtifactError> {
        ValidatedContract::validate(self)
    }

    /// Encode exact canonical marker bytes without a trailing newline.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, ArtifactError> {
        canonical_contract_bytes(self)
    }

    /// Decode canonical bytes and require the exact declared ownership.
    pub fn decode_and_validate(bytes: &[u8], expected: &Self) -> Result<Self, ArtifactError> {
        decode_contract(bytes, expected)
    }
}

impl ValidatedContract for SandboxMarker {
    fn validate(&self) -> Result<(), ArtifactError> {
        validate_header(
            self.schema,
            self.kind,
            OwnershipMarkerKind::Sandbox,
            &self.namespace,
        )?;
        self.roots.validate(self.platform)?;
        validate_profile_sandbox_root(self.platform, self.roots.root(), &self.profile)
    }
}

trait ValidatedContract: DeserializeOwned + Eq + Serialize {
    fn validate(&self) -> Result<(), ArtifactError>;
}

fn canonical_contract_bytes<T: ValidatedContract>(value: &T) -> Result<Vec<u8>, ArtifactError> {
    value.validate()?;
    canonical_json_bytes(&serde_json::to_value(value)?)
}

fn decode_contract<T: ValidatedContract>(bytes: &[u8], expected: &T) -> Result<T, ArtifactError> {
    expected.validate()?;
    let value: Value = serde_json::from_slice(bytes)?;
    if canonical_json_bytes(&value)? != bytes {
        return Err(ArtifactError::new("sandbox evidence is not canonical JSON"));
    }
    let decoded: T = serde_json::from_value(value)?;
    decoded.validate()?;
    if &decoded != expected {
        return Err(ArtifactError::new(
            "sandbox evidence does not match its declared value",
        ));
    }
    Ok(decoded)
}

fn validate_header(
    schema: u16,
    kind: OwnershipMarkerKind,
    expected_kind: OwnershipMarkerKind,
    namespace: &str,
) -> Result<(), ArtifactError> {
    require(
        schema == OWNERSHIP_SCHEMA,
        "ownership marker schema is invalid",
    )?;
    require(kind == expected_kind, "ownership marker kind is invalid")?;
    require(
        namespace == STABLE_SANDBOX_NAMESPACE,
        "ownership marker namespace is invalid",
    )
}

/// Observed ownership evidence at one sandbox path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxEvidenceState {
    /// The path is absent.
    Absent,
    /// The path has valid typed ownership evidence.
    Valid,
    /// The path exists but its ownership evidence is invalid.
    Invalid,
}

/// Required action for one original/cleanup evidence pair.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum QuarantineDecision {
    /// Create a fresh sandbox.
    FreshCreate,
    /// Quarantine and clean the valid original sandbox.
    CleanupOriginal,
    /// Recover and remove the valid cleanup quarantine before recreation.
    RecoverCleanup,
    /// Refuse and retain every existing path.
    RefuseRetainAll,
}

/// Decide the complete nine-state sandbox cleanup policy.
#[must_use]
pub const fn decide_quarantine(
    original: SandboxEvidenceState,
    cleanup: SandboxEvidenceState,
) -> QuarantineDecision {
    match (original, cleanup) {
        (SandboxEvidenceState::Absent, SandboxEvidenceState::Absent) => {
            QuarantineDecision::FreshCreate
        }
        (SandboxEvidenceState::Valid, SandboxEvidenceState::Absent) => {
            QuarantineDecision::CleanupOriginal
        }
        (SandboxEvidenceState::Absent, SandboxEvidenceState::Valid) => {
            QuarantineDecision::RecoverCleanup
        }
        (SandboxEvidenceState::Valid, SandboxEvidenceState::Valid)
        | (SandboxEvidenceState::Valid, SandboxEvidenceState::Invalid)
        | (SandboxEvidenceState::Invalid, SandboxEvidenceState::Valid)
        | (SandboxEvidenceState::Invalid, SandboxEvidenceState::Absent)
        | (SandboxEvidenceState::Absent, SandboxEvidenceState::Invalid)
        | (SandboxEvidenceState::Invalid, SandboxEvidenceState::Invalid) => {
            QuarantineDecision::RefuseRetainAll
        }
    }
}

/// Return the namespace-relative profile lease path.
#[must_use]
pub fn profile_lease_path(profile: &SafeProfileId) -> PathBuf {
    PathBuf::from(LEASE_DIRECTORY).join(format!("{profile}.lock"))
}

/// Return the namespace-relative profile sandbox path.
#[must_use]
pub fn profile_sandbox_path(profile: &SafeProfileId) -> PathBuf {
    PathBuf::from(SANDBOX_DIRECTORY).join(profile.as_str())
}

/// Return the namespace-relative cleanup quarantine path.
#[must_use]
pub fn profile_cleanup_path(profile: &SafeProfileId) -> PathBuf {
    PathBuf::from(SANDBOX_DIRECTORY).join(format!(".cleanup-{profile}"))
}
