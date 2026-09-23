//! The stable profile sandbox: candidates, cleanup, quarantine, and the sandbox owner.

use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
    fs, io,
    path::{Path, PathBuf},
};

use skit_tui_walker_support::sandbox::{
    QuarantineDecision, SafeProfileId, SandboxEvidenceState, SandboxMetadata, decide_quarantine,
};
use tempfile::TempDir;

use super::stable_namespace::{
    SandboxError, SandboxFaultPoint, SandboxFaults, StableSandboxNamespace, StableSandboxPaths,
    acquire_profile_lease_retained, combine_sandbox_release, create_relative_directory_with_fault,
    current_sandbox_platform, initialize_namespace_retained, literal_path, publish_relative_marker,
    release_profile_lease, require_exact_names, require_ticket, ticket_named,
    validate_relative_marker,
};
use crate::cli::tui_real_sandbox_fs::{
    ChildName, EntryTicket, NamespaceHandles, NodeKind as SandboxNodeKind, PinnedDirectory,
    PinnedFile, ProfileLease, SandboxFsError, ValidatedCleanupSandbox, ValidatedOriginalSandbox,
};

#[derive(Debug)]
pub(super) enum Evidence<T> {
    Absent,
    Valid(T),
    Invalid { reason: String },
}

impl<T> Evidence<T> {
    pub(super) const fn state(&self) -> SandboxEvidenceState {
        match self {
            Self::Absent => SandboxEvidenceState::Absent,
            Self::Valid(_) => SandboxEvidenceState::Valid,
            Self::Invalid { .. } => SandboxEvidenceState::Invalid,
        }
    }

    pub(super) fn reason(&self) -> Option<&str> {
        match self {
            Self::Invalid { reason } => Some(reason),
            Self::Absent | Self::Valid(_) => None,
        }
    }

    pub(super) fn into_valid(self) -> Option<T> {
        match self {
            Self::Valid(value) => Some(value),
            Self::Absent | Self::Invalid { .. } => None,
        }
    }
}

#[derive(Debug)]
pub(super) struct CandidateParts {
    root: PinnedDirectory,
    marker: PinnedFile,
    children: BTreeMap<ChildName, PinnedDirectory>,
    tickets: Vec<EntryTicket>,
}

fn open_candidate_parts(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<CandidateParts, SandboxError> {
    open_candidate_parts_with_hooks(
        parent,
        name,
        paths,
        no_candidate_tickets_hook,
        no_candidate_root_hook,
    )
}

pub(super) fn no_candidate_tickets_hook(_: &PinnedDirectory, _: &[EntryTicket]) {}

pub(super) fn no_candidate_root_hook(_: &PinnedDirectory, _: &PinnedDirectory, _: &ChildName) {}

pub(super) fn open_candidate_parts_with_hooks(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
    after_tickets: impl FnOnce(&PinnedDirectory, &[EntryTicket]),
    before_final_root_ticket: impl FnOnce(&PinnedDirectory, &PinnedDirectory, &ChildName),
) -> Result<CandidateParts, SandboxError> {
    let root = parent.open_directory(name).map_err(SandboxError::from)?;
    require_ticket(parent, name, SandboxNodeKind::Directory, root.identity())?;
    let tickets = root.tickets().map_err(SandboxError::from)?;
    after_tickets(&root, &tickets);
    let mut children = BTreeMap::new();
    for child_name in paths.sandbox_child_names() {
        let ticket = tickets.iter().find(|ticket| ticket.name() == &child_name);
        let Some(ticket) = ticket else {
            continue;
        };
        if ticket.kind() != SandboxNodeKind::Directory {
            return Err(SandboxError::invalid(
                &root.path().join(child_name.as_os_str()),
                "the sandbox child root is not an ordinary directory",
            ));
        }
        let child = root
            .open_directory(&child_name)
            .map_err(SandboxError::from)?;
        if child.identity() != ticket.identity() {
            return Err(SandboxError::invalid(
                child.path(),
                "the sandbox child root changed after enumeration",
            ));
        }
        children.insert(child_name, child);
    }
    let marker = validate_relative_marker(
        &root,
        &paths.sandbox_marker_name(),
        paths.sandbox_marker_bytes(),
    )?;
    root.verify_at(parent, name).map_err(SandboxError::from)?;
    marker
        .verify_at(&root, &paths.sandbox_marker_name())
        .map_err(SandboxError::from)?;
    before_final_root_ticket(parent, &root, name);
    require_ticket(parent, name, SandboxNodeKind::Directory, root.identity())?;
    Ok(CandidateParts {
        root,
        marker,
        children,
        tickets,
    })
}

pub(super) fn validate_original_candidate(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    let CandidateParts {
        root,
        marker,
        children,
        tickets: _,
    } = open_candidate_parts(parent, name, paths)?;
    let expected = paths.sandbox_child_names();
    let sandbox = ValidatedOriginalSandbox::new_original(root, marker, expected.clone(), children)
        .map_err(SandboxError::from)?;
    let mut allowed = expected.to_vec();
    allowed.push(paths.sandbox_marker_name());
    require_exact_names(sandbox.root(), &allowed)?;
    Ok(sandbox)
}

pub(super) fn validate_cleanup_candidate(
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<ValidatedCleanupSandbox, SandboxError> {
    let CandidateParts {
        root,
        marker,
        children,
        tickets,
    } = open_candidate_parts(parent, name, paths)?;
    let mut allowed = paths.sandbox_child_names().to_vec();
    allowed.push(paths.sandbox_marker_name());
    if tickets
        .iter()
        .any(|ticket| !allowed.iter().any(|allowed| allowed == ticket.name()))
    {
        return Err(SandboxError::invalid(
            root.path(),
            "the cleanup directory has an unexpected child",
        ));
    }
    Ok(ValidatedCleanupSandbox::new_cleanup(root, marker, children))
}

pub(super) fn classify_retained_sandbox<T>(
    parent: &PinnedDirectory,
    name: &ChildName,
    validate: impl FnOnce(&PinnedDirectory, &ChildName) -> Result<T, SandboxError>,
) -> Result<Evidence<T>, SandboxError> {
    if ticket_named(parent, name)?.is_none() {
        return Ok(Evidence::Absent);
    }
    match validate(parent, name) {
        Ok(sandbox) => Ok(Evidence::Valid(sandbox)),
        Err(error @ SandboxError::InvalidEvidence { .. }) => Ok(Evidence::Invalid {
            reason: error.to_string(),
        }),
        Err(error) => Err(error),
    }
}

fn create_fresh_sandbox_retained(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    create_fresh_sandbox_retained_with_hooks(
        paths,
        handles,
        faults,
        no_fresh_sandbox_before_marker_hook,
        no_fresh_sandbox_after_marker_hook,
    )
}

pub(super) fn no_fresh_sandbox_before_marker_hook(_: &PinnedDirectory, _: &ChildName) {}

pub(super) fn no_fresh_sandbox_after_marker_hook(_: &PinnedDirectory) {}

pub(super) fn create_fresh_sandbox_retained_with_hooks(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
    before_marker_publish: impl FnOnce(&PinnedDirectory, &ChildName),
    after_marker_publish: impl FnOnce(&PinnedDirectory),
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    let root = create_relative_directory_with_fault(
        handles.sandboxes(),
        &paths.sandbox_name(),
        &paths.sandbox_root,
        "create sandbox",
        faults,
        SandboxFaultPoint::CreateSandboxRootIo,
    )?;
    let mut children = BTreeMap::new();
    for child_name in paths.sandbox_child_names() {
        let child_path = root.path().join(child_name.as_os_str());
        let child = create_relative_directory_with_fault(
            &root,
            &child_name,
            &child_path,
            "create sandbox child",
            faults,
            SandboxFaultPoint::CreateSandboxChildIo,
        )?;
        child.sync().map_err(SandboxError::from)?;
        children.insert(child_name, child);
    }
    root.sync().map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::BeforeSandboxMarker)?;
    let marker_name = paths.sandbox_marker_name();
    before_marker_publish(&root, &marker_name);
    let marker = publish_relative_marker(&root, &marker_name, paths.sandbox_marker_bytes())?;
    root.sync().map_err(SandboxError::from)?;
    after_marker_publish(&root);
    let sandbox =
        ValidatedOriginalSandbox::new_original(root, marker, paths.sandbox_child_names(), children)
            .expect("seven created child roots produce original sandbox evidence");
    validate_same_original(&sandbox, handles.sandboxes(), &paths.sandbox_name(), paths)?;
    faults.check(SandboxFaultPoint::AfterSandboxMarker)?;
    Ok(sandbox)
}

fn validate_same_candidate_core(
    root: &PinnedDirectory,
    marker: &PinnedFile,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    root.verify_handle().map_err(SandboxError::from)?;
    root.verify_at(parent, name).map_err(SandboxError::from)?;
    require_ticket(parent, name, SandboxNodeKind::Directory, root.identity())?;
    marker.verify_handle().map_err(SandboxError::from)?;
    marker
        .verify_at(root, &paths.sandbox_marker_name())
        .map_err(SandboxError::from)?;
    let marker_bytes = marker
        .read_all(root, &paths.sandbox_marker_name())
        .map_err(SandboxError::from)?;
    if marker_bytes != paths.sandbox_marker_bytes() {
        return Err(SandboxError::invalid(
            marker.path(),
            "the marker bytes are invalid",
        ));
    }
    Ok(())
}

fn validate_same_original(
    sandbox: &ValidatedOriginalSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    validate_same_original_with_hook(sandbox, parent, name, paths, no_candidate_child_hook)
}

fn no_candidate_child_hook(_: &PinnedDirectory, _: &ChildName, _: &PinnedDirectory) {}

pub(super) fn validate_same_original_with_hook(
    sandbox: &ValidatedOriginalSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
    mut after_child_verify: impl FnMut(&PinnedDirectory, &ChildName, &PinnedDirectory),
) -> Result<(), SandboxError> {
    validate_same_candidate_core(sandbox.root(), sandbox.marker(), parent, name, paths)?;
    for (child_name, child) in sandbox.children() {
        child.verify_handle().map_err(SandboxError::from)?;
        child
            .verify_at(sandbox.root(), child_name)
            .map_err(SandboxError::from)?;
        after_child_verify(sandbox.root(), child_name, child);
        require_ticket(
            sandbox.root(),
            child_name,
            SandboxNodeKind::Directory,
            child.identity(),
        )?;
    }
    let mut allowed = sandbox
        .children()
        .iter()
        .map(|(name, _)| name.clone())
        .collect::<Vec<_>>();
    allowed.push(paths.sandbox_marker_name());
    require_exact_names(sandbox.root(), &allowed)
}

fn validate_same_cleanup(
    sandbox: &ValidatedCleanupSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
) -> Result<(), SandboxError> {
    validate_same_cleanup_with_hook(sandbox, parent, name, paths, no_candidate_child_hook)
}

pub(super) fn validate_same_cleanup_with_hook(
    sandbox: &ValidatedCleanupSandbox,
    parent: &PinnedDirectory,
    name: &ChildName,
    paths: &StableSandboxPaths,
    mut after_child_verify: impl FnMut(&PinnedDirectory, &ChildName, &PinnedDirectory),
) -> Result<(), SandboxError> {
    validate_same_candidate_core(sandbox.root(), sandbox.marker(), parent, name, paths)?;
    for (child_name, child) in sandbox.children() {
        child.verify_handle().map_err(SandboxError::from)?;
        child
            .verify_at(sandbox.root(), child_name)
            .map_err(SandboxError::from)?;
        after_child_verify(sandbox.root(), child_name, child);
        require_ticket(
            sandbox.root(),
            child_name,
            SandboxNodeKind::Directory,
            child.identity(),
        )?;
    }
    let mut allowed = sandbox.children().keys().cloned().collect::<Vec<_>>();
    allowed.push(paths.sandbox_marker_name());
    require_exact_names(sandbox.root(), &allowed)
}

fn remove_cleanup_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    remove_cleanup_sandbox_with_hooks(
        paths,
        handles,
        sandbox,
        faults,
        CleanupHooks {
            after_plan: no_cleanup_plan_hook,
            after_children: no_cleanup_marker_hook,
            after_marker_read: no_cleanup_marker_hook,
            before_root_ticket: no_cleanup_root_hook,
        },
    )
}

pub(super) fn no_cleanup_plan_hook(_: &PinnedDirectory) {}

pub(super) fn no_cleanup_marker_hook(_: &PinnedDirectory, _: &PinnedFile) {}

pub(super) fn no_cleanup_root_hook(_: &PinnedDirectory, _: &PinnedDirectory, _: &ChildName) {}

pub(super) struct CleanupHooks<AfterPlan, AfterChildren, AfterMarkerRead, BeforeRootTicket> {
    pub(super) after_plan: AfterPlan,
    pub(super) after_children: AfterChildren,
    pub(super) after_marker_read: AfterMarkerRead,
    pub(super) before_root_ticket: BeforeRootTicket,
}

pub(super) fn remove_cleanup_sandbox_with_hooks<
    AfterPlan,
    AfterChildren,
    AfterMarkerRead,
    BeforeRootTicket,
>(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
    hooks: CleanupHooks<AfterPlan, AfterChildren, AfterMarkerRead, BeforeRootTicket>,
) -> Result<(), SandboxError>
where
    AfterPlan: FnOnce(&PinnedDirectory),
    AfterChildren: FnOnce(&PinnedDirectory, &PinnedFile),
    AfterMarkerRead: FnOnce(&PinnedDirectory, &PinnedFile),
    BeforeRootTicket: FnOnce(&PinnedDirectory, &PinnedDirectory, &ChildName),
{
    let CleanupHooks {
        after_plan,
        after_children,
        after_marker_read,
        before_root_ticket,
    } = hooks;
    let cleanup_name = paths.cleanup_name();
    let sandbox = sandbox.into_cleanup_plan();
    after_plan(sandbox.root());
    let marker_name = paths.sandbox_marker_name();
    for ticket in sandbox.root().tickets().map_err(SandboxError::from)? {
        if ticket.name() == &marker_name {
            continue;
        }
        let expected = sandbox.children().get(ticket.name()).ok_or_else(|| {
            SandboxError::invalid(
                &sandbox.root().path().join(ticket.name().as_os_str()),
                "the cleanup child is not one retained sandbox root",
            )
        })?;
        if ticket.kind() != SandboxNodeKind::Directory || ticket.identity() != *expected {
            return Err(SandboxError::invalid(
                &sandbox.root().path().join(ticket.name().as_os_str()),
                "the cleanup child changed after validation",
            ));
        }
        faults.check(SandboxFaultPoint::RemoveChild)?;
        sandbox
            .root()
            .remove_tree(ticket)
            .map_err(SandboxError::from)?;
        faults.check(SandboxFaultPoint::AfterChildRemoval)?;
    }
    require_exact_names(sandbox.root(), std::slice::from_ref(&marker_name))?;
    after_children(sandbox.root(), sandbox.marker());
    sandbox
        .marker()
        .verify_handle()
        .map_err(SandboxError::from)?;
    let marker_bytes = sandbox
        .marker()
        .read_all(sandbox.root(), &marker_name)
        .map_err(SandboxError::from)?;
    if marker_bytes != paths.sandbox_marker_bytes() {
        return Err(SandboxError::invalid(
            sandbox.marker().path(),
            "the marker bytes are invalid",
        ));
    }
    after_marker_read(sandbox.root(), sandbox.marker());
    let marker_ticket = ticket_named(sandbox.root(), &marker_name)?.ok_or_else(|| {
        SandboxError::invalid(sandbox.marker().path(), "the sandbox marker is missing")
    })?;
    if marker_ticket.kind() != SandboxNodeKind::RegularFile
        || marker_ticket.identity() != sandbox.marker().identity()
    {
        return Err(SandboxError::invalid(
            sandbox.marker().path(),
            "the sandbox marker changed before removal",
        ));
    }
    faults.check(SandboxFaultPoint::RemoveMarker)?;
    let root = sandbox.into_root();
    root.remove_ticket(marker_ticket)
        .map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::RemoveDirectory)?;
    root.verify_handle().map_err(SandboxError::from)?;
    before_root_ticket(handles.sandboxes(), &root, &cleanup_name);
    let root_ticket = ticket_named(handles.sandboxes(), &cleanup_name)?
        .ok_or_else(|| SandboxError::invalid(&paths.cleanup_root, "the cleanup root is missing"))?;
    if root_ticket.kind() != SandboxNodeKind::Directory || root_ticket.identity() != root.identity()
    {
        return Err(SandboxError::invalid(
            &paths.cleanup_root,
            "the cleanup root changed before removal",
        ));
    }
    drop(root);
    handles
        .sandboxes()
        .remove_ticket(root_ticket)
        .map_err(SandboxError::from)
}

fn quarantine_original_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    source_name: ChildName,
    sandbox: ValidatedOriginalSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    quarantine_original_sandbox_with_hook(
        paths,
        handles,
        source_name,
        sandbox,
        faults,
        no_quarantine_after_rename_hook,
    )
}

fn no_quarantine_after_rename_hook(_: &PinnedDirectory, _: &ChildName) {}

pub(super) fn quarantine_original_sandbox_with_hook(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    source_name: ChildName,
    sandbox: ValidatedOriginalSandbox,
    faults: &SandboxFaults,
    after_rename: impl FnOnce(&PinnedDirectory, &ChildName),
) -> Result<(), SandboxError> {
    let identity = sandbox.root().identity();
    let cleanup_name = paths.cleanup_name();
    faults.check(SandboxFaultPoint::RenameOriginal)?;
    let ticket = sandbox.into_rename_ticket(source_name, cleanup_name.clone());
    let moved = handles
        .sandboxes()
        .rename_noreplace(ticket)
        .map_err(SandboxError::from)?;
    drop(moved);
    handles.sandboxes().sync().map_err(SandboxError::from)?;
    faults.check(SandboxFaultPoint::AfterRenameIdentity)?;
    after_rename(handles.sandboxes(), &cleanup_name);
    let cleanup = validate_cleanup_candidate(handles.sandboxes(), &cleanup_name, paths)?;
    if cleanup.root().identity() != identity {
        return Err(SandboxError::invalid(
            &paths.cleanup_root,
            "the reopened cleanup identity does not match the rename source",
        ));
    }
    remove_cleanup_sandbox(paths, handles, cleanup, faults)
}

fn recover_cleanup_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    recover_cleanup_sandbox_with_hook(paths, handles, sandbox, faults, no_recover_cleanup_hook)
}

fn no_recover_cleanup_hook(_: &ValidatedCleanupSandbox) {}

pub(super) fn recover_cleanup_sandbox_with_hook(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedCleanupSandbox,
    faults: &SandboxFaults,
    before_validate: impl FnOnce(&ValidatedCleanupSandbox),
) -> Result<(), SandboxError> {
    before_validate(&sandbox);
    validate_same_cleanup(&sandbox, handles.sandboxes(), &paths.cleanup_name(), paths)?;
    remove_cleanup_sandbox(paths, handles, sandbox, faults)
}

fn refuse_retained_quarantine_state<O, C>(
    paths: &StableSandboxPaths,
    original: &Evidence<O>,
    cleanup: &Evidence<C>,
) -> SandboxError {
    SandboxError::invalid(
        &paths.sandboxes_root,
        format!(
            "refused original {:?} and cleanup {:?}; original detail: {}; cleanup detail: {}",
            original.state(),
            cleanup.state(),
            original.reason().unwrap_or("none"),
            cleanup.reason().unwrap_or("none"),
        ),
    )
}

pub(super) fn prepare_fresh_sandbox_retained(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    faults: &SandboxFaults,
) -> Result<ValidatedOriginalSandbox, SandboxError> {
    let original = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.sandbox_name(),
        |parent, name| validate_original_candidate(parent, name, paths),
    )?;
    let cleanup = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.cleanup_name(),
        |parent, name| validate_cleanup_candidate(parent, name, paths),
    )?;
    match decide_quarantine(original.state(), cleanup.state()) {
        QuarantineDecision::FreshCreate => {}
        QuarantineDecision::CleanupOriginal => quarantine_original_sandbox(
            paths,
            handles,
            paths.sandbox_name(),
            original
                .into_valid()
                .expect("the cleanup-original decision has valid original evidence"),
            faults,
        )?,
        QuarantineDecision::RecoverCleanup => recover_cleanup_sandbox(
            paths,
            handles,
            cleanup
                .into_valid()
                .expect("the recover-cleanup decision has valid cleanup evidence"),
            faults,
        )?,
        QuarantineDecision::RefuseRetainAll => {
            return Err(refuse_retained_quarantine_state(paths, &original, &cleanup));
        }
    }
    create_fresh_sandbox_retained(paths, handles, faults)
}

fn close_retained_sandbox(
    paths: &StableSandboxPaths,
    handles: &NamespaceHandles,
    sandbox: ValidatedOriginalSandbox,
    faults: &SandboxFaults,
) -> Result<(), SandboxError> {
    validate_same_original(&sandbox, handles.sandboxes(), &paths.sandbox_name(), paths)?;
    let cleanup = classify_retained_sandbox(
        handles.sandboxes(),
        &paths.cleanup_name(),
        |parent, name| validate_cleanup_candidate(parent, name, paths),
    )?;
    if cleanup.state() != SandboxEvidenceState::Absent {
        let original = Evidence::Valid(());
        return Err(refuse_retained_quarantine_state(paths, &original, &cleanup));
    }
    quarantine_original_sandbox(paths, handles, paths.sandbox_name(), sandbox, faults)
}

#[derive(Debug)]
pub(super) struct StableSandbox {
    pub(super) paths: StableSandboxPaths,
    namespace: Option<NamespaceHandles>,
    pub(super) namespace_marker: Option<PinnedFile>,
    pub(super) lease: Option<ProfileLease>,
    pub(super) sandbox: Option<ValidatedOriginalSandbox>,
    faults: SandboxFaults,
    armed: bool,
}

fn no_live_namespace_marker_hook(_: &PinnedDirectory, _: &PinnedFile, _: &ChildName) {}

pub(super) fn no_acquisition_after_prepare_hook(_: &StableSandbox) {}

fn combine_acquisition_abort(
    primary: SandboxError,
    cleanup: Result<(), SandboxError>,
    release: Result<(), SandboxError>,
) -> SandboxError {
    let primary = match cleanup {
        Ok(()) => primary,
        Err(cleanup) => SandboxError::Dual {
            primary: Box::new(primary),
            release: Box::new(cleanup),
        },
    };
    match release {
        Ok(()) => primary,
        Err(release) => SandboxError::Dual {
            primary: Box::new(primary),
            release: Box::new(release),
        },
    }
}

impl StableSandbox {
    pub(super) fn acquire(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
        faults: SandboxFaults,
    ) -> Result<Self, SandboxError> {
        Self::acquire_with_hooks(
            namespace,
            profile,
            faults,
            no_acquisition_after_prepare_hook,
            release_profile_lease,
        )
    }

    pub(super) fn acquire_with_hooks(
        namespace: StableSandboxNamespace,
        profile: SafeProfileId,
        faults: SandboxFaults,
        after_prepare: impl FnOnce(&StableSandbox),
        abort_release: impl FnOnce(ProfileLease, &PinnedDirectory) -> Result<(), SandboxFsError>,
    ) -> Result<Self, SandboxError> {
        let paths = StableSandboxPaths::new(namespace, profile)?;
        let (namespace, namespace_marker) = initialize_namespace_retained(&paths, &faults)?;
        let lease = acquire_profile_lease_retained(&paths, &namespace, &faults)?;
        let mut sandbox = Self {
            paths,
            namespace: Some(namespace),
            namespace_marker: Some(namespace_marker),
            lease: Some(lease),
            sandbox: None,
            faults,
            armed: true,
        };
        let prepared = match prepare_fresh_sandbox_retained(
            &sandbox.paths,
            sandbox.handles(),
            &sandbox.faults,
        ) {
            Ok(prepared) => prepared,
            Err(primary) => {
                return Err(sandbox.abort_acquire(primary, abort_release));
            }
        };
        sandbox.sandbox = Some(prepared);
        after_prepare(&sandbox);
        let validation = sandbox
            .validate_namespace()
            .and_then(|()| sandbox.validate_lease())
            .and_then(|()| sandbox.validate_live_sandbox());
        if let Err(primary) = validation {
            return Err(sandbox.abort_acquire(primary, abort_release));
        }
        Ok(sandbox)
    }

    fn abort_acquire(
        mut self,
        primary: SandboxError,
        release: impl FnOnce(ProfileLease, &PinnedDirectory) -> Result<(), SandboxFsError>,
    ) -> SandboxError {
        self.armed = false;
        let cleanup = match self.sandbox.take() {
            Some(sandbox) => {
                close_retained_sandbox(&self.paths, self.handles(), sandbox, &self.faults)
            }
            None => Ok(()),
        };
        let release = release(
            self.lease
                .take()
                .expect("an armed partial sandbox retains its profile lease"),
            self.handles().leases(),
        )
        .map_err(SandboxError::from);
        combine_acquisition_abort(primary, cleanup, release)
    }

    pub(super) fn path(&self) -> &Path {
        &self.paths.sandbox_root
    }

    pub(super) fn metadata(
        &self,
        review_profile: &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError> {
        if review_profile != &self.paths.profile {
            return Err(SandboxError::invalid(
                &self.paths.sandbox_root,
                "the requested review profile does not own this sandbox",
            ));
        }
        self.validate_namespace()?;
        self.validate_lease()?;
        self.validate_live_sandbox()?;
        SandboxMetadata::stable(
            self.paths.platform,
            literal_path(&self.paths.namespace_root)?,
            BTreeSet::from([review_profile.clone()]),
        )
        .map_err(|error| SandboxError::invalid(&self.paths.namespace_root, error.to_string()))
    }

    pub(super) fn handles(&self) -> &NamespaceHandles {
        self.namespace
            .as_ref()
            .expect("an armed stable sandbox retains its namespace handles")
    }

    pub(super) fn validate_namespace(&self) -> Result<(), SandboxError> {
        self.validate_namespace_with_hook(no_live_namespace_marker_hook)
    }

    pub(super) fn validate_namespace_with_hook(
        &self,
        after_marker_read: impl FnOnce(&PinnedDirectory, &PinnedFile, &ChildName),
    ) -> Result<(), SandboxError> {
        let handles = self.handles();
        handles
            .verify(
                &self.paths.namespace_name(),
                &self.paths.leases_name(),
                &self.paths.sandboxes_name(),
            )
            .map_err(SandboxError::from)?;
        let marker = self
            .namespace_marker
            .as_ref()
            .expect("an armed stable sandbox retains its namespace marker");
        marker.verify_handle().map_err(SandboxError::from)?;
        marker
            .verify_at(handles.namespace(), &self.paths.namespace_marker_name())
            .map_err(SandboxError::from)?;
        let bytes = marker
            .read_all(handles.namespace(), &self.paths.namespace_marker_name())
            .map_err(SandboxError::from)?;
        if bytes != self.paths.namespace_marker_bytes() {
            return Err(SandboxError::invalid(
                marker.path(),
                "the namespace marker bytes are invalid",
            ));
        }
        let marker_name = self.paths.namespace_marker_name();
        after_marker_read(handles.namespace(), marker, &marker_name);
        require_ticket(
            handles.namespace(),
            &marker_name,
            SandboxNodeKind::RegularFile,
            marker.identity(),
        )?;
        require_exact_names(
            handles.namespace(),
            &[
                self.paths.namespace_marker_name(),
                self.paths.leases_name(),
                self.paths.sandboxes_name(),
            ],
        )
    }

    pub(super) fn validate_live_sandbox(&self) -> Result<(), SandboxError> {
        validate_same_original(
            self.sandbox
                .as_ref()
                .expect("an armed stable sandbox retains its validated root"),
            self.handles().sandboxes(),
            &self.paths.sandbox_name(),
            &self.paths,
        )
    }

    pub(super) fn validate_lease(&self) -> Result<(), SandboxError> {
        let lease = self
            .lease
            .as_ref()
            .expect("an armed stable sandbox retains its profile lease");
        lease
            .validate(self.handles().leases())
            .map_err(SandboxError::from)?;
        if lease
            .file()
            .read_all(self.handles().leases(), &self.paths.lease_name())
            .map_err(SandboxError::from)?
            != self.paths.lease_marker_bytes()
        {
            return Err(SandboxError::invalid(
                &self.paths.lease_path,
                "the held profile lease marker bytes are invalid",
            ));
        }
        Ok(())
    }

    pub(super) fn close(mut self) -> Result<(), SandboxError> {
        self.armed = false;
        let primary = (|| {
            self.validate_namespace()?;
            self.validate_lease()?;
            self.validate_live_sandbox()?;
            let sandbox = self
                .sandbox
                .take()
                .expect("an armed stable sandbox retains its validated root");
            close_retained_sandbox(&self.paths, self.handles(), sandbox, &self.faults)
        })();
        let release = self
            .lease
            .take()
            .expect("an armed stable sandbox retains its profile lease")
            .release(self.handles().leases())
            .map_err(SandboxError::from);
        combine_sandbox_release(primary, release)
    }
}

impl Drop for StableSandbox {
    fn drop(&mut self) {
        if self.armed {
            self.armed = false;
            let can_cleanup = self.validate_namespace().is_ok()
                && self.validate_lease().is_ok()
                && self.validate_live_sandbox().is_ok();
            if can_cleanup && let Some(sandbox) = self.sandbox.take() {
                let _ = close_retained_sandbox(&self.paths, self.handles(), sandbox, &self.faults);
            }
            if let Some(lease) = self.lease.take() {
                let _ = lease.release(self.handles().leases());
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum SandboxOwner {
    Random { directory: TempDir, root: PathBuf },
    Stable(Box<StableSandbox>),
}

/// The prefix that Windows `fs::canonicalize` puts in front of a resolved drive path.
const VERBATIM_PREFIX: &str = r"\\?\";

/// The prefix that the verbatim form puts in front of a resolved network-share path.
const VERBATIM_UNC_PREFIX: &str = r"\\?\UNC\";

/// Whether one text starts with a drive letter, a colon, and a separator.
fn starts_with_drive(text: &str) -> bool {
    let mut characters = text.chars();
    matches!(
        (characters.next(), characters.next(), characters.next()),
        (Some(letter), Some(':'), Some('\\')) if letter.is_ascii_alphabetic()
    )
}

/// Rewrite one Windows verbatim path into the ordinary spelling of the same location.
///
/// `\\?\C:\a\b` becomes `C:\a\b`, and `\\?\UNC\server\share\a` becomes `\\server\share\a`. Every
/// other text stays as it is, because only these two verbatim forms have an ordinary spelling. A
/// device path such as `\\?\Volume{...}` has none.
///
/// The rule reads text only, so it runs and it is testable on every platform. It is correct for
/// the output of `fs::canonicalize`. Do not use it on a hand-written verbatim path: the verbatim
/// form keeps a trailing dot or space that the ordinary form loses.
pub(super) fn ordinary_windows_path(text: &str) -> Cow<'_, str> {
    if let Some(share) = text.strip_prefix(VERBATIM_UNC_PREFIX) {
        return Cow::Owned(format!(r"\\{share}"));
    }
    match text.strip_prefix(VERBATIM_PREFIX) {
        Some(rest) if starts_with_drive(rest) => Cow::Borrowed(rest),
        _ => Cow::Borrowed(text),
    }
}

/// Get one path in the spelling that this fixture looks up and compares.
///
/// The fixture speaks the ordinary spelling. A path that the product recorded can arrive in the
/// verbatim form, so it drops that prefix first. A path that is not Unicode keeps its exact bytes.
pub(super) fn ordinary_path(path: &Path) -> Cow<'_, Path> {
    match path.to_str().map(ordinary_windows_path) {
        Some(Cow::Borrowed(text)) => Cow::Borrowed(Path::new(text)),
        Some(Cow::Owned(text)) => Cow::Owned(PathBuf::from(text)),
        None => Cow::Borrowed(path),
    }
}

/// Whether one path that the product recorded names the same location as one fixture path.
///
/// The product resolves every path it records, and Windows `fs::canonicalize` returns the
/// verbatim form. The recorded text therefore differs from the fixture spelling on Windows even
/// when both name one file, so the comparison drops the verbatim prefix first.
pub(super) fn names_the_same_path(recorded: &str, fixture: &Path) -> bool {
    *ordinary_path(Path::new(recorded)) == *fixture
}

/// Get the random sandbox root in the spelling that the product records.
///
/// The product resolves every source path it records, so the fixture root resolves too. If it does
/// not, the projection cannot recognize the stored path. A Unix temporary directory can sit behind
/// a symlink (macOS does this for every one; Linux does when `TMPDIR` points at a link). A Windows
/// temporary directory can carry an 8.3 short name: the GitHub runner gives
/// `C:\Users\RUNNER~1\AppData\Local\Temp`, and `fs::canonicalize` expands it to `runneradmin`.
///
/// Windows must resolve the root too, but it must not keep the resolved spelling. Windows
/// `fs::canonicalize` returns the `\\?\` verbatim form, and that spelling must stay out of the
/// recorded interface. The root therefore drops the prefix, and `ordinary_path` drops it again
/// from every path that the product recorded. Both sides then speak the ordinary spelling.
///
/// A resolved root hides one product behavior: for an owned draft the product keeps two spellings
/// on purpose (`skit-ui/src/add.rs`: `path` keeps skit's own spelling, `source_record` holds the
/// resolved one). The walker oracle `project_commit_review_name` requires the two to name one
/// location, so no walker run exercises the two-spelling case.
fn resolved_random_root(root: &Path) -> PathBuf {
    let resolved = fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    ordinary_path(&resolved).into_owned()
}

impl SandboxOwner {
    pub(super) fn random() -> Result<Self, SandboxError> {
        Self::random_with(TempDir::new)
    }

    pub(super) fn random_with(
        create: impl FnOnce() -> io::Result<TempDir>,
    ) -> Result<Self, SandboxError> {
        create()
            .map(|directory| {
                let root = resolved_random_root(directory.path());
                Self::Random { directory, root }
            })
            .map_err(|error| {
                SandboxError::io("create random sandbox", Path::new("<system-temp>"), error)
            })
    }

    pub(super) fn path(&self) -> &Path {
        match self {
            Self::Random { root, .. } => root.as_path(),
            Self::Stable(sandbox) => sandbox.path(),
        }
    }

    pub(super) fn stable(&self) -> Option<&StableSandbox> {
        match self {
            Self::Random { .. } => None,
            Self::Stable(sandbox) => Some(sandbox),
        }
    }

    pub(super) fn metadata(
        &self,
        review_profile: &SafeProfileId,
    ) -> Result<SandboxMetadata, SandboxError> {
        match self {
            Self::Random { root, .. } => {
                let platform = current_sandbox_platform()?;
                let literal = literal_path(root)?;
                SandboxMetadata::random(platform, literal, review_profile.clone())
                    .map_err(|error| SandboxError::invalid(root, error.to_string()))
            }
            Self::Stable(sandbox) => sandbox.metadata(review_profile),
        }
    }

    pub(super) fn close(self) -> Result<(), SandboxError> {
        match self {
            Self::Random { directory, root } => directory
                .close()
                .map_err(|error| SandboxError::io("close random sandbox", &root, error)),
            Self::Stable(sandbox) => StableSandbox::close(*sandbox),
        }
    }
}
