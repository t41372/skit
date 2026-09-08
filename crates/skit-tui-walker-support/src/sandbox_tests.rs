use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use crate::{
    bundle::{
        RunMetadata, bundle_digest, chunk_json_path, chunk_view_path, decode_run_metadata,
        profile_directory, run_metadata_bytes, validate_run_metadata,
    },
    canonical_json_bytes,
    sandbox::{
        NamespaceMarker, ParentInitLockMarker, ProfileLeaseMarker, QuarantineDecision,
        SafeProfileId, SandboxEvidenceState, SandboxMarker, SandboxMetadata, SandboxMode,
        SandboxPlatform, SandboxRoots, decide_quarantine, profile_cleanup_path, profile_lease_path,
        profile_sandbox_path,
    },
    validate_stable_namespace_root,
};

const WINDOWS_INVALID_LITERAL_ROOTS: &[&str] = &[
    r"C:\Temp.\skit-ui-walker-v1",
    r"C:\Temp \skit-ui-walker-v1",
    r"C:\CON\skit-ui-walker-v1",
    r"C:\con.txt\skit-ui-walker-v1",
    r"C:\PrN.any\skit-ui-walker-v1",
    r"C:\AUX.log\skit-ui-walker-v1",
    r"C:\nul.tmp\skit-ui-walker-v1",
    r"C:\com1.log\skit-ui-walker-v1",
    r"C:\COM9\skit-ui-walker-v1",
    r"C:\LPT1\skit-ui-walker-v1",
    r"C:\lpt9.tmp\skit-ui-walker-v1",
    r"\\?\C:\Temp\skit-ui-walker-v1",
    r"\\?\UNC\server\share\skit-ui-walker-v1",
    r"\\.\C:\Temp\skit-ui-walker-v1",
    r"C:\Temp:alias\skit-ui-walker-v1",
    r"\\server\share:alias\skit-ui-walker-v1",
    r"C:\bad<name\skit-ui-walker-v1",
    r"C:\bad>name\skit-ui-walker-v1",
    r#"C:\bad"name\skit-ui-walker-v1"#,
    r"C:\bad|name\skit-ui-walker-v1",
    r"C:\bad?name\skit-ui-walker-v1",
    r"C:\bad*name\skit-ui-walker-v1",
    r"C:\COM¹\skit-ui-walker-v1",
    r"C:\com².log\skit-ui-walker-v1",
    r"C:\LPT³.tmp\skit-ui-walker-v1",
];

fn profile(value: &str) -> SafeProfileId {
    SafeProfileId::try_from(value).unwrap()
}

fn sandbox_roots(platform: SandboxPlatform, root: &str) -> SandboxRoots {
    SandboxRoots::new(platform, root).unwrap()
}

fn canonical_edit(bytes: &[u8], edit: impl FnOnce(&mut serde_json::Map<String, Value>)) -> Vec<u8> {
    let mut value: Value = serde_json::from_slice(bytes).unwrap();
    edit(value.as_object_mut().unwrap());
    canonical_json_bytes(&value).unwrap()
}

#[test]
fn safe_profile_id_accepts_boundaries_and_final_smoke_profiles() {
    let accepted = [
        "a".to_owned(),
        "0".to_owned(),
        format!("a{}z", "-".repeat(62)),
        "a".repeat(64),
        "engine-smoke-60x24".to_owned(),
        "en-80x24".to_owned(),
        "zh-cn-120x30".to_owned(),
        "zh-tw-40x40".to_owned(),
        "pseudo-120x12".to_owned(),
    ];
    for raw in accepted {
        let parsed = SafeProfileId::try_from(raw.clone()).unwrap();
        assert_eq!(raw.parse::<SafeProfileId>().unwrap(), parsed);
        assert_eq!(parsed.as_str(), raw);
        assert_eq!(parsed.to_string(), raw);
        assert_eq!(
            serde_json::to_string(&parsed).unwrap(),
            format!("\"{raw}\"")
        );
        assert_eq!(
            serde_json::from_str::<SafeProfileId>(&format!("\"{raw}\"")).unwrap(),
            parsed
        );
    }
    assert!(serde_json::from_str::<SafeProfileId>("7").is_err());
}

#[test]
fn safe_profile_id_rejects_every_grammar_class() {
    let too_long = "a".repeat(65);
    for raw in [
        "",
        too_long.as_str(),
        "-leading",
        "trailing-",
        "Upper",
        "under_score",
        "dot.name",
        "with space",
        "with/slash",
        "with\\slash",
        "with:colon",
        "é",
        "a\nb",
        ".",
        "..",
    ] {
        assert!(SafeProfileId::try_from(raw).is_err(), "accepted {raw:?}");
        assert!(
            serde_json::from_str::<SafeProfileId>(&serde_json::to_string(raw).unwrap()).is_err(),
            "deserialized {raw:?}"
        );
    }
    assert_eq!(
        SafeProfileId::try_from("").unwrap_err().to_string(),
        "profile id must contain 1 through 64 ASCII bytes"
    );
    assert_eq!(
        SafeProfileId::try_from("Upper").unwrap_err().to_string(),
        "profile id must use lowercase ASCII letters, digits, and interior hyphens"
    );
}

#[test]
fn safe_profile_id_rejects_all_reserved_device_names() {
    for raw in [
        "con", "prn", "aux", "nul", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8",
        "com9", "lpt1", "lpt2", "lpt3", "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
    ] {
        assert!(SafeProfileId::try_from(raw).is_err(), "accepted {raw}");
    }
    for raw in ["com0", "com10", "lpt0", "lpt10", "console", "auxiliary"] {
        assert_eq!(profile(raw).as_str(), raw);
    }
    assert_eq!(
        SafeProfileId::try_from("con").unwrap_err().to_string(),
        "profile id is a reserved device name"
    );
}

#[test]
fn typed_ownership_markers_have_exact_canonical_bytes() {
    let profile = profile("en-80x24");
    let parent =
        ParentInitLockMarker::new(SandboxPlatform::Linux, "/tmp/skit-ui-walker-v1").unwrap();
    let namespace = NamespaceMarker::new(SandboxPlatform::Linux, "/tmp/skit-ui-walker-v1").unwrap();
    let lease = ProfileLeaseMarker::new(
        SandboxPlatform::Linux,
        profile.clone(),
        "/tmp/skit-ui-walker-v1",
    )
    .unwrap();
    let sandbox =
        SandboxMarker::new(SandboxPlatform::Linux, profile, "/tmp/skit-ui-walker-v1").unwrap();

    let parent_bytes = parent.canonical_bytes().unwrap();
    assert_eq!(
        parent_bytes,
        br#"{"init_lock":"/tmp/.skit-ui-walker-v1.init.lock","kind":"parent_init_lock","namespace":"skit-ui-walker-v1","namespace_root":"/tmp/skit-ui-walker-v1","platform":"linux","schema":1}"#
    );
    let namespace_bytes = namespace.canonical_bytes().unwrap();
    assert_eq!(
        namespace_bytes,
        br#"{"kind":"namespace","leases_root":"/tmp/skit-ui-walker-v1/leases","namespace":"skit-ui-walker-v1","platform":"linux","root":"/tmp/skit-ui-walker-v1","sandboxes_root":"/tmp/skit-ui-walker-v1/sandboxes","schema":1}"#
    );
    let lease_bytes = lease.canonical_bytes().unwrap();
    assert_eq!(
        lease_bytes,
        br#"{"cleanup_root":"/tmp/skit-ui-walker-v1/sandboxes/.cleanup-en-80x24","kind":"profile_lease","lease_path":"/tmp/skit-ui-walker-v1/leases/en-80x24.lock","namespace":"skit-ui-walker-v1","namespace_root":"/tmp/skit-ui-walker-v1","platform":"linux","profile":"en-80x24","sandbox_root":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24","schema":1}"#
    );
    let sandbox_bytes = sandbox.canonical_bytes().unwrap();
    assert_eq!(
        sandbox_bytes,
        br#"{"kind":"sandbox","namespace":"skit-ui-walker-v1","platform":"linux","profile":"en-80x24","roots":{"config":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/config","cwd":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/cwd","data":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/data","external":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/external","home":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/home","root":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24","state":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/state","system_temp":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24/system-temp"},"schema":1}"#
    );
    for bytes in [
        &parent_bytes,
        &namespace_bytes,
        &lease_bytes,
        &sandbox_bytes,
    ] {
        assert!(!bytes.ends_with(b"\n"));
    }
    assert_eq!(lease.profile().as_str(), "en-80x24");
    assert_eq!(sandbox.profile().as_str(), "en-80x24");
    let roots = sandbox.roots();
    assert_eq!(
        [
            roots.root(),
            roots.data(),
            roots.state(),
            roots.config(),
            roots.home(),
            roots.cwd(),
            roots.external(),
            roots.system_temp(),
        ],
        [
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/data",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/state",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/config",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/home",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/cwd",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/external",
            "/tmp/skit-ui-walker-v1/sandboxes/en-80x24/system-temp",
        ]
    );
    assert_eq!(
        ParentInitLockMarker::decode_and_validate(&parent_bytes, &parent).unwrap(),
        parent
    );
    assert_eq!(
        NamespaceMarker::decode_and_validate(&namespace_bytes, &namespace).unwrap(),
        namespace
    );
    assert_eq!(
        ProfileLeaseMarker::decode_and_validate(&lease_bytes, &lease).unwrap(),
        lease
    );
    assert_eq!(
        SandboxMarker::decode_and_validate(&sandbox_bytes, &sandbox).unwrap(),
        sandbox
    );
}

#[test]
fn declared_layout_is_byte_exact_for_linux_macos_and_windows() {
    let linux = sandbox_roots(SandboxPlatform::Linux, "/tmp/random-sandbox");
    assert_eq!(linux.data(), "/tmp/random-sandbox/data");
    assert_eq!(linux.system_temp(), "/tmp/random-sandbox/system-temp");
    let linux_metadata = SandboxMetadata::random(
        SandboxPlatform::Linux,
        "/tmp/random-sandbox",
        profile("en-80x24"),
    )
    .unwrap();
    assert_eq!(
        linux_metadata.canonical_bytes().unwrap(),
        br#"{"mode":"random","namespace":"skit-ui-walker-v1","platform":"linux","profiles":{"en-80x24":"/tmp/random-sandbox"},"root":"/tmp/random-sandbox","schema":1}"#
    );

    let macos = sandbox_roots(SandboxPlatform::Macos, "/private/tmp/random-sandbox");
    assert_eq!(macos.cwd(), "/private/tmp/random-sandbox/cwd");
    assert_eq!(macos.external(), "/private/tmp/random-sandbox/external");
    let macos_metadata = SandboxMetadata::random(
        SandboxPlatform::Macos,
        "/private/tmp/random-sandbox",
        profile("en-80x24"),
    )
    .unwrap();
    assert_eq!(
        macos_metadata.canonical_bytes().unwrap(),
        br#"{"mode":"random","namespace":"skit-ui-walker-v1","platform":"macos","profiles":{"en-80x24":"/private/tmp/random-sandbox"},"root":"/private/tmp/random-sandbox","schema":1}"#
    );

    let windows = sandbox_roots(SandboxPlatform::Windows, r"C:\Temp\random-sandbox");
    assert_eq!(windows.home(), r"C:\Temp\random-sandbox\home");
    assert_eq!(windows.state(), r"C:\Temp\random-sandbox\state");
    let windows_metadata = SandboxMetadata::random(
        SandboxPlatform::Windows,
        r"C:\Temp\random-sandbox",
        profile("en-80x24"),
    )
    .unwrap();
    assert_eq!(
        windows_metadata.canonical_bytes().unwrap(),
        br#"{"mode":"random","namespace":"skit-ui-walker-v1","platform":"windows","profiles":{"en-80x24":"C:\\Temp\\random-sandbox"},"root":"C:\\Temp\\random-sandbox","schema":1}"#
    );
    let windows_unc = sandbox_roots(SandboxPlatform::Windows, r"\\server\share\random-sandbox");
    assert_eq!(windows_unc.data(), r"\\server\share\random-sandbox\data");

    let parent =
        ParentInitLockMarker::new(SandboxPlatform::Windows, r"C:\Temp\skit-ui-walker-v1").unwrap();
    assert_eq!(
        parent.canonical_bytes().unwrap(),
        br#"{"init_lock":"C:\\Temp\\.skit-ui-walker-v1.init.lock","kind":"parent_init_lock","namespace":"skit-ui-walker-v1","namespace_root":"C:\\Temp\\skit-ui-walker-v1","platform":"windows","schema":1}"#
    );
    let unix_root_parent =
        ParentInitLockMarker::new(SandboxPlatform::Linux, "/skit-ui-walker-v1").unwrap();
    assert_eq!(
        unix_root_parent.canonical_bytes().unwrap(),
        br#"{"init_lock":"/.skit-ui-walker-v1.init.lock","kind":"parent_init_lock","namespace":"skit-ui-walker-v1","namespace_root":"/skit-ui-walker-v1","platform":"linux","schema":1}"#
    );
    let volume_parent =
        ParentInitLockMarker::new(SandboxPlatform::Windows, r"C:\skit-ui-walker-v1").unwrap();
    assert_eq!(
        volume_parent.canonical_bytes().unwrap(),
        br#"{"init_lock":"C:\\.skit-ui-walker-v1.init.lock","kind":"parent_init_lock","namespace":"skit-ui-walker-v1","namespace_root":"C:\\skit-ui-walker-v1","platform":"windows","schema":1}"#
    );
    let lease = ProfileLeaseMarker::new(
        SandboxPlatform::Windows,
        profile("en-80x24"),
        r"C:\Temp\skit-ui-walker-v1",
    )
    .unwrap();
    assert_eq!(
        lease.canonical_bytes().unwrap(),
        br#"{"cleanup_root":"C:\\Temp\\skit-ui-walker-v1\\sandboxes\\.cleanup-en-80x24","kind":"profile_lease","lease_path":"C:\\Temp\\skit-ui-walker-v1\\leases\\en-80x24.lock","namespace":"skit-ui-walker-v1","namespace_root":"C:\\Temp\\skit-ui-walker-v1","platform":"windows","profile":"en-80x24","sandbox_root":"C:\\Temp\\skit-ui-walker-v1\\sandboxes\\en-80x24","schema":1}"#
    );
    for component in [
        "com0", "COM10", "lpt0", "LPT10", "company", "laptop", "com⁴", "lpt¹0",
    ] {
        let root = format!(r"C:\{component}\skit-ui-walker-v1");
        NamespaceMarker::new(SandboxPlatform::Windows, root).unwrap();
    }
}

#[test]
fn marker_constructors_reject_invalid_or_aliased_declared_roots() {
    for root in [
        "relative",
        "/tmp/not-the-namespace",
        "/tmp/skit-ui-walker-v1/",
        "/tmp/./skit-ui-walker-v1",
        "/tmp/parent/../skit-ui-walker-v1",
        "/tmp//skit-ui-walker-v1",
    ] {
        assert!(
            ParentInitLockMarker::new(SandboxPlatform::Linux, root).is_err(),
            "accepted parent root {root:?}"
        );
        assert!(
            NamespaceMarker::new(SandboxPlatform::Linux, root).is_err(),
            "accepted namespace root {root:?}"
        );
    }
    for root in [
        r"C:/Temp/skit-ui-walker-v1",
        r"C:\Temp\skit-ui-walker-v1\",
        r"C:\Temp\.\skit-ui-walker-v1",
        r"C:\Temp\\skit-ui-walker-v1",
        r"c:\Temp\skit-ui-walker-v1",
    ] {
        assert!(
            NamespaceMarker::new(SandboxPlatform::Windows, root).is_err(),
            "accepted Windows alias {root:?}"
        );
    }
    for root in WINDOWS_INVALID_LITERAL_ROOTS {
        assert!(
            NamespaceMarker::new(SandboxPlatform::Windows, *root).is_err(),
            "accepted reserved Windows root {root:?}"
        );
    }
    for prefix in ["COM", "lpt"] {
        for digit in '1'..='9' {
            let root = format!(r"C:\{prefix}{digit}.log\skit-ui-walker-v1");
            assert!(
                NamespaceMarker::new(SandboxPlatform::Windows, &root).is_err(),
                "accepted Windows device alias {root:?}"
            );
        }
    }
    for prefix in ["com", "LPT"] {
        for digit in ['¹', '²', '³'] {
            let root = format!(r"C:\{prefix}{digit}.log\skit-ui-walker-v1");
            assert!(
                NamespaceMarker::new(SandboxPlatform::Windows, &root).is_err(),
                "accepted superscript Windows device alias {root:?}"
            );
        }
    }
    for control in 1..=31 {
        let root = format!("C:\\bad{}name\\skit-ui-walker-v1", char::from(control));
        assert!(
            NamespaceMarker::new(SandboxPlatform::Windows, &root).is_err(),
            "accepted Windows control byte {control}"
        );
    }
    assert!(SandboxRoots::new(SandboxPlatform::Linux, "/tmp/root/").is_err());
    assert!(SandboxRoots::new(SandboxPlatform::Linux, "/tmp/root/../other").is_err());
    assert!(SandboxRoots::new(SandboxPlatform::Windows, r"\\server").is_err());
}

#[test]
fn stable_namespace_root_validator_reuses_the_closed_literal_grammar() {
    for (platform, root) in [
        (SandboxPlatform::Linux, "/tmp/skit-ui-walker-v1"),
        (SandboxPlatform::Windows, r"C:\Temp\skit-ui-walker-v1"),
        (
            SandboxPlatform::Windows,
            r"\\server\share\skit-ui-walker-v1",
        ),
    ] {
        validate_stable_namespace_root(platform, root).unwrap();
    }
    assert!(
        validate_stable_namespace_root(SandboxPlatform::Macos, "/tmp/skit-ui-walker-v1").is_err()
    );

    for root in [
        "relative/skit-ui-walker-v1",
        "/tmp/not-the-namespace",
        "/tmp/skit-ui-walker-v1/",
        "/tmp//skit-ui-walker-v1",
        "/tmp/./skit-ui-walker-v1",
        "/tmp/parent/../skit-ui-walker-v1",
        "/tmp/newline\n/skit-ui-walker-v1",
        "/tmp/carriage\r/skit-ui-walker-v1",
        "/tmp/nul\0/skit-ui-walker-v1",
    ] {
        assert!(
            validate_stable_namespace_root(SandboxPlatform::Linux, root).is_err(),
            "accepted Linux namespace root {root:?}"
        );
    }
    for root in [
        r"c:\Temp\skit-ui-walker-v1",
        r"C:/Temp/skit-ui-walker-v1",
        r"C:\Temp\\skit-ui-walker-v1",
        r"C:\Temp\skit-ui-walker-v1\",
        r"C:\Temp\.\skit-ui-walker-v1",
        r"C:\Temp\parent\..\skit-ui-walker-v1",
        "C:\\bad\nname\\skit-ui-walker-v1",
        r"C:\CON\skit-ui-walker-v1",
        r"C:\Temp:stream\skit-ui-walker-v1",
    ] {
        assert!(
            validate_stable_namespace_root(SandboxPlatform::Windows, root).is_err(),
            "accepted Windows namespace root {root:?}"
        );
    }
}

#[test]
fn marker_decode_refuses_noncanonical_unknown_partial_and_wrong_evidence() {
    let expected = SandboxMarker::new(
        SandboxPlatform::Linux,
        profile("en-80x24"),
        "/tmp/skit-ui-walker-v1",
    )
    .unwrap();
    let bytes = expected.canonical_bytes().unwrap();

    let mut with_newline = bytes.clone();
    with_newline.push(b'\n');
    assert!(SandboxMarker::decode_and_validate(&with_newline, &expected).is_err());
    let mut with_space = vec![b' '];
    with_space.extend_from_slice(&bytes);
    assert!(SandboxMarker::decode_and_validate(&with_space, &expected).is_err());
    let reordered = format!(
        "{{\"schema\":1,\"kind\":\"sandbox\",\"namespace\":\"skit-ui-walker-v1\",\"platform\":\"linux\",\"profile\":\"en-80x24\",\"roots\":{}}}",
        serde_json::to_string(expected.roots()).unwrap()
    );
    assert!(SandboxMarker::decode_and_validate(reordered.as_bytes(), &expected).is_err());
    let unknown = canonical_edit(&bytes, |object| {
        object.insert("unknown".to_owned(), json!(true));
    });
    assert!(SandboxMarker::decode_and_validate(&unknown, &expected).is_err());
    let partial = canonical_edit(&bytes, |object| {
        object.remove("roots");
    });
    assert!(SandboxMarker::decode_and_validate(&partial, &expected).is_err());

    for (field, wrong) in [
        ("schema", json!(2)),
        ("kind", json!("namespace")),
        ("namespace", json!("other")),
        ("platform", json!("windows")),
        ("profile", json!("zh-cn-120x30")),
    ] {
        let corrupt = canonical_edit(&bytes, |object| {
            object.insert(field.to_owned(), wrong);
        });
        assert!(
            SandboxMarker::decode_and_validate(&corrupt, &expected).is_err(),
            "accepted wrong {field}"
        );
    }
    for (field, wrong) in [
        ("schema", json!(2)),
        ("kind", json!("namespace")),
        ("namespace", json!("other")),
    ] {
        let corrupt = canonical_edit(&bytes, |object| {
            object.insert(field.to_owned(), wrong);
        });
        let marker: SandboxMarker = serde_json::from_slice(&corrupt).unwrap();
        assert!(marker.validate().is_err(), "validated wrong {field}");
    }
    let wrong_root = canonical_edit(&bytes, |object| {
        object["roots"].as_object_mut().unwrap().insert(
            "root".to_owned(),
            json!("/tmp/skit-ui-walker-v1/sandboxes/other"),
        );
    });
    assert!(SandboxMarker::decode_and_validate(&wrong_root, &expected).is_err());

    for replacement in [
        "/tmp/skit-ui-walker-v1/sandboxes/other",
        "/tmp/skit-ui-walker-v1/other-directory/en-80x24",
        "/tmp/other-namespace/sandboxes/en-80x24",
    ] {
        let internally_consistent_wrong_root = canonical_edit(&bytes, |object| {
            for value in object["roots"].as_object_mut().unwrap().values_mut() {
                let path = value.as_str().unwrap();
                *value =
                    json!(path.replace("/tmp/skit-ui-walker-v1/sandboxes/en-80x24", replacement,));
            }
        });
        let marker: SandboxMarker =
            serde_json::from_slice(&internally_consistent_wrong_root).unwrap();
        assert!(
            marker.canonical_bytes().is_err(),
            "accepted sandbox root {replacement:?}"
        );
    }

    for field in [
        "data",
        "state",
        "config",
        "home",
        "cwd",
        "external",
        "system_temp",
    ] {
        let wrong_child = canonical_edit(&bytes, |object| {
            object["roots"]
                .as_object_mut()
                .unwrap()
                .insert(field.to_owned(), json!(format!("/tmp/wrong/{field}")));
        });
        let marker: SandboxMarker = serde_json::from_slice(&wrong_child).unwrap();
        assert!(
            marker.canonical_bytes().is_err(),
            "accepted wrong sandbox child {field}"
        );
    }

    let parent =
        ParentInitLockMarker::new(SandboxPlatform::Linux, "/tmp/skit-ui-walker-v1").unwrap();
    for (field, wrong) in [
        ("schema", json!(2)),
        ("kind", json!("namespace")),
        ("namespace", json!("other")),
    ] {
        let corrupt = canonical_edit(&parent.canonical_bytes().unwrap(), |object| {
            object.insert(field.to_owned(), wrong);
        });
        let corrupt: ParentInitLockMarker = serde_json::from_slice(&corrupt).unwrap();
        assert!(
            corrupt.canonical_bytes().is_err(),
            "accepted wrong parent marker {field}"
        );
    }
    let wrong_parent = canonical_edit(&parent.canonical_bytes().unwrap(), |object| {
        object.insert("init_lock".to_owned(), json!("/tmp/other.init.lock"));
    });
    let wrong_parent: ParentInitLockMarker = serde_json::from_slice(&wrong_parent).unwrap();
    assert!(wrong_parent.canonical_bytes().is_err());

    let namespace = NamespaceMarker::new(SandboxPlatform::Linux, "/tmp/skit-ui-walker-v1").unwrap();
    for (field, wrong) in [
        ("schema", json!(2)),
        ("kind", json!("sandbox")),
        ("namespace", json!("other")),
    ] {
        let corrupt = canonical_edit(&namespace.canonical_bytes().unwrap(), |object| {
            object.insert(field.to_owned(), wrong);
        });
        let corrupt: NamespaceMarker = serde_json::from_slice(&corrupt).unwrap();
        assert!(
            corrupt.canonical_bytes().is_err(),
            "accepted wrong namespace marker {field}"
        );
    }
    for field in ["leases_root", "sandboxes_root"] {
        let wrong_child = canonical_edit(&namespace.canonical_bytes().unwrap(), |object| {
            object.insert(field.to_owned(), json!(format!("/tmp/wrong/{field}")));
        });
        let wrong_child: NamespaceMarker = serde_json::from_slice(&wrong_child).unwrap();
        assert!(
            wrong_child.canonical_bytes().is_err(),
            "accepted wrong namespace child {field}"
        );
    }

    let lease = ProfileLeaseMarker::new(
        SandboxPlatform::Linux,
        profile("en-80x24"),
        "/tmp/skit-ui-walker-v1",
    )
    .unwrap();
    for (field, wrong) in [
        ("schema", json!(2)),
        ("kind", json!("namespace")),
        ("namespace", json!("other")),
    ] {
        let corrupt = canonical_edit(&lease.canonical_bytes().unwrap(), |object| {
            object.insert(field.to_owned(), wrong);
        });
        let corrupt: ProfileLeaseMarker = serde_json::from_slice(&corrupt).unwrap();
        assert!(
            corrupt.canonical_bytes().is_err(),
            "accepted wrong lease marker {field}"
        );
    }
    for field in ["lease_path", "sandbox_root", "cleanup_root"] {
        let wrong_child = canonical_edit(&lease.canonical_bytes().unwrap(), |object| {
            object.insert(field.to_owned(), json!(format!("/tmp/wrong/{field}")));
        });
        let wrong_child: ProfileLeaseMarker = serde_json::from_slice(&wrong_child).unwrap();
        assert!(
            wrong_child.canonical_bytes().is_err(),
            "accepted wrong lease child {field}"
        );
    }

    let other = SandboxMarker::new(
        SandboxPlatform::Linux,
        profile("zh-cn-120x30"),
        "/tmp/skit-ui-walker-v1",
    )
    .unwrap();
    assert_eq!(
        SandboxMarker::decode_and_validate(&other.canonical_bytes().unwrap(), &expected)
            .unwrap_err()
            .to_string(),
        "sandbox evidence does not match its declared value"
    );
}

fn assert_quarantine(
    original: SandboxEvidenceState,
    cleanup: SandboxEvidenceState,
    expected: QuarantineDecision,
) {
    assert_eq!(decide_quarantine(original, cleanup), expected);
}

#[test]
fn quarantine_absent_absent_creates_fresh() {
    assert_quarantine(
        SandboxEvidenceState::Absent,
        SandboxEvidenceState::Absent,
        QuarantineDecision::FreshCreate,
    );
}

#[test]
fn quarantine_valid_absent_cleans_original() {
    assert_quarantine(
        SandboxEvidenceState::Valid,
        SandboxEvidenceState::Absent,
        QuarantineDecision::CleanupOriginal,
    );
}

#[test]
fn quarantine_absent_valid_recovers_cleanup() {
    assert_quarantine(
        SandboxEvidenceState::Absent,
        SandboxEvidenceState::Valid,
        QuarantineDecision::RecoverCleanup,
    );
}

#[test]
fn quarantine_valid_valid_refuses() {
    assert_quarantine(
        SandboxEvidenceState::Valid,
        SandboxEvidenceState::Valid,
        QuarantineDecision::RefuseRetainAll,
    );
}

#[test]
fn quarantine_valid_invalid_refuses() {
    assert_quarantine(
        SandboxEvidenceState::Valid,
        SandboxEvidenceState::Invalid,
        QuarantineDecision::RefuseRetainAll,
    );
}

#[test]
fn quarantine_invalid_valid_refuses() {
    assert_quarantine(
        SandboxEvidenceState::Invalid,
        SandboxEvidenceState::Valid,
        QuarantineDecision::RefuseRetainAll,
    );
}

#[test]
fn quarantine_invalid_absent_refuses() {
    assert_quarantine(
        SandboxEvidenceState::Invalid,
        SandboxEvidenceState::Absent,
        QuarantineDecision::RefuseRetainAll,
    );
}

#[test]
fn quarantine_absent_invalid_refuses() {
    assert_quarantine(
        SandboxEvidenceState::Absent,
        SandboxEvidenceState::Invalid,
        QuarantineDecision::RefuseRetainAll,
    );
}

#[test]
fn quarantine_invalid_invalid_refuses() {
    assert_quarantine(
        SandboxEvidenceState::Invalid,
        SandboxEvidenceState::Invalid,
        QuarantineDecision::RefuseRetainAll,
    );
}

#[test]
fn safe_profile_ids_are_sorted_map_keys() {
    let mut profiles = BTreeMap::new();
    profiles.insert(profile("zh-tw-40x40"), "third");
    profiles.insert(profile("en-80x24"), "first");
    profiles.insert(profile("zh-cn-120x30"), "second");
    assert_eq!(
        profiles
            .keys()
            .map(SafeProfileId::as_str)
            .collect::<Vec<_>>(),
        ["en-80x24", "zh-cn-120x30", "zh-tw-40x40"]
    );
}

fn sandbox_metadata(
    platform: SandboxPlatform,
    root: &str,
    profile_ids: &[&str],
) -> SandboxMetadata {
    let profile_ids = profile_ids
        .iter()
        .map(|profile_id| profile(profile_id))
        .collect::<BTreeSet<_>>();
    SandboxMetadata::stable(platform, root, profile_ids).unwrap()
}

fn sandbox_metadata_with_mode(
    mode: SandboxMode,
    platform: SandboxPlatform,
    root: &str,
    profile_ids: &[&str],
) -> SandboxMetadata {
    match mode {
        SandboxMode::Random => {
            assert_eq!(profile_ids.len(), 1);
            SandboxMetadata::random(platform, root, profile(profile_ids[0])).unwrap()
        }
        SandboxMode::Stable => sandbox_metadata(platform, root, profile_ids),
    }
}

#[test]
fn sandbox_metadata_round_trips_every_mode_and_platform() {
    for platform in [
        SandboxPlatform::Linux,
        SandboxPlatform::Macos,
        SandboxPlatform::Windows,
    ] {
        for mode in [SandboxMode::Random, SandboxMode::Stable] {
            if mode == SandboxMode::Stable && platform == SandboxPlatform::Macos {
                assert!(
                    SandboxMetadata::stable(
                        platform,
                        "/private/tmp/skit-ui-walker-v1",
                        BTreeSet::from([profile("en-80x24")]),
                    )
                    .is_err()
                );
                continue;
            }
            let (root, profile_root) = match platform {
                SandboxPlatform::Linux => (
                    "/tmp/skit-ui-walker-v1",
                    "/tmp/skit-ui-walker-v1/sandboxes/en-80x24",
                ),
                SandboxPlatform::Macos => (
                    "/private/tmp/skit-ui-walker-v1",
                    "/private/tmp/skit-ui-walker-v1/sandboxes/en-80x24",
                ),
                SandboxPlatform::Windows => (
                    r"C:\Temp\skit-ui-walker-v1",
                    r"C:\Temp\skit-ui-walker-v1\sandboxes\en-80x24",
                ),
            };
            let metadata = sandbox_metadata_with_mode(mode, platform, root, &["en-80x24"]);
            assert_eq!(metadata.mode(), mode);
            assert_eq!(metadata.platform(), platform);
            assert_eq!(
                metadata.profiles()[&profile("en-80x24")],
                if mode == SandboxMode::Random {
                    root
                } else {
                    profile_root
                }
            );
            let bytes = metadata.canonical_bytes().unwrap();
            assert_eq!(
                SandboxMetadata::decode_and_validate(&bytes, &metadata).unwrap(),
                metadata
            );
        }
    }
}

fn run_metadata() -> RunMetadata {
    let sandbox = sandbox_metadata(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        &["en-80x24"],
    );
    RunMetadata {
        schema: 2,
        source_revision: "revision".to_owned(),
        deterministic_result: "passed".to_owned(),
        operation_count: 100,
        operations_sha256: "a".repeat(64),
        final_liveness_sha256: "b".repeat(64),
        profiles: vec![profile("en-80x24")],
        sandbox,
    }
}

#[test]
fn sandbox_metadata_and_schema_two_run_have_exact_canonical_bytes() {
    let run = run_metadata();
    assert_eq!(run.sandbox.namespace(), "skit-ui-walker-v1");
    assert_eq!(run.sandbox.root(), "/tmp/skit-ui-walker-v1");
    let sandbox_bytes = run.sandbox.canonical_bytes().unwrap();
    assert_eq!(
        sandbox_bytes,
        br#"{"mode":"stable","namespace":"skit-ui-walker-v1","platform":"linux","profiles":{"en-80x24":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24"},"root":"/tmp/skit-ui-walker-v1","schema":1}"#
    );
    assert!(!sandbox_bytes.ends_with(b"\n"));
    assert_eq!(
        SandboxMetadata::decode_and_validate(&sandbox_bytes, &run.sandbox).unwrap(),
        run.sandbox
    );

    let run_bytes = run_metadata_bytes(&run).unwrap();
    assert_eq!(
        canonical_json_bytes(&serde_json::to_value(&run).unwrap()).unwrap(),
        run_bytes
    );
    let debug = format!("{run:?}");
    assert!(debug.contains("profiles: [SafeProfileId(\"en-80x24\")]"));
    assert!(debug.contains("sandbox: SandboxMetadata"));
    assert_eq!(
        run_bytes,
        br#"{"deterministic_result":"passed","final_liveness_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","operation_count":100,"operations_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","profiles":["en-80x24"],"sandbox":{"mode":"stable","namespace":"skit-ui-walker-v1","platform":"linux","profiles":{"en-80x24":"/tmp/skit-ui-walker-v1/sandboxes/en-80x24"},"root":"/tmp/skit-ui-walker-v1","schema":1},"schema":2,"source_revision":"revision"}"#
    );
    assert!(!run_bytes.ends_with(b"\n"));
    assert_eq!(decode_run_metadata(&run_bytes).unwrap(), run);
}

#[test]
fn run_metadata_rejects_old_schemas_before_missing_schema_two_fields() {
    let schema_one = br#"{"deterministic_result":"passed","final_liveness_sha256":"bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb","operation_count":100,"operations_sha256":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","profiles":["en-80x24"],"schema":1,"source_revision":"revision"}"#;
    for schema in [0, 1, 3] {
        let bytes = canonical_edit(schema_one, |object| {
            object.insert("schema".to_owned(), json!(schema));
        });
        assert_eq!(
            decode_run_metadata(&bytes).unwrap_err().to_string(),
            "run metadata has an unsupported schema"
        );
    }

    let schema_two = canonical_edit(schema_one, |object| {
        object.insert("schema".to_owned(), json!(2));
    });
    assert!(
        decode_run_metadata(&schema_two)
            .unwrap_err()
            .to_string()
            .contains("missing field `sandbox`")
    );

    let mut unknown = serde_json::to_value(run_metadata()).unwrap();
    unknown
        .as_object_mut()
        .unwrap()
        .insert("unknown".to_owned(), json!(true));
    assert!(serde_json::from_value::<RunMetadata>(unknown).is_err());
}

#[test]
fn sandbox_metadata_refuses_every_invalid_field_and_noncanonical_shape() {
    let expected = run_metadata().sandbox;
    let bytes = expected.canonical_bytes().unwrap();
    assert!(
        SandboxMetadata::stable(
            SandboxPlatform::Linux,
            "/tmp/skit-ui-walker-v1",
            BTreeSet::new(),
        )
        .is_err()
    );
    for invalid_root in [r"1:\Temp\root", r"C-\Temp\root", r"C:relative", "C:"] {
        assert!(
            SandboxMetadata::random(SandboxPlatform::Windows, invalid_root, profile("a"),).is_err(),
            "accepted invalid Windows root {invalid_root:?}"
        );
    }
    for invalid_root in WINDOWS_INVALID_LITERAL_ROOTS {
        assert!(
            SandboxMetadata::random(SandboxPlatform::Windows, *invalid_root, profile("a"),)
                .is_err(),
            "accepted reserved Windows metadata root {invalid_root:?}"
        );
    }
    for prefix in ["COM", "lpt"] {
        for digit in '1'..='9' {
            let root = format!(r"C:\{prefix}{digit}.log\random");
            assert!(
                SandboxMetadata::random(SandboxPlatform::Windows, &root, profile("a")).is_err(),
                "accepted Windows metadata device alias {root:?}"
            );
        }
    }
    for prefix in ["com", "LPT"] {
        for digit in ['¹', '²', '³'] {
            let root = format!(r"C:\{prefix}{digit}.log\random");
            assert!(
                SandboxMetadata::random(SandboxPlatform::Windows, &root, profile("a")).is_err(),
                "accepted superscript metadata device alias {root:?}"
            );
        }
    }
    for control in 1..=31 {
        let root = format!("C:\\bad{}name\\random", char::from(control));
        assert!(
            SandboxMetadata::random(SandboxPlatform::Windows, &root, profile("a")).is_err(),
            "accepted Windows metadata control byte {control}"
        );
    }
    assert!(
        SandboxMetadata::random(SandboxPlatform::Windows, "/tmp/not-windows", profile("a"),)
            .is_err()
    );
    assert!(SandboxMetadata::random(SandboxPlatform::Linux, "/tmp/root\n", profile("a"),).is_err());
    assert!(
        SandboxMetadata::stable(
            SandboxPlatform::Linux,
            "/tmp/root",
            BTreeSet::from([profile("a")]),
        )
        .is_err()
    );
    assert!(
        SandboxMetadata::stable(
            SandboxPlatform::Linux,
            "relative",
            BTreeSet::from([profile("a")]),
        )
        .is_err()
    );

    let mut with_newline = bytes.clone();
    with_newline.push(b'\n');
    assert!(SandboxMetadata::decode_and_validate(&with_newline, &expected).is_err());
    let unknown = canonical_edit(&bytes, |object| {
        object.insert("unknown".to_owned(), json!(true));
    });
    assert!(SandboxMetadata::decode_and_validate(&unknown, &expected).is_err());
    let partial = canonical_edit(&bytes, |object| {
        object.remove("profiles");
    });
    assert!(SandboxMetadata::decode_and_validate(&partial, &expected).is_err());
    for (field, wrong) in [
        ("schema", json!(2)),
        ("mode", json!("random")),
        ("namespace", json!("other")),
        ("platform", json!("windows")),
        ("root", json!("/tmp/other")),
    ] {
        let corrupt = canonical_edit(&bytes, |object| {
            object.insert(field.to_owned(), wrong);
        });
        assert!(
            SandboxMetadata::decode_and_validate(&corrupt, &expected).is_err(),
            "accepted wrong {field}"
        );
    }
    let wrong_key = canonical_edit(&bytes, |object| {
        let profiles = object["profiles"].as_object_mut().unwrap();
        let root = profiles.remove("en-80x24").unwrap();
        profiles.insert("Upper".to_owned(), root);
    });
    assert!(SandboxMetadata::decode_and_validate(&wrong_key, &expected).is_err());
    let wrong_value = canonical_edit(&bytes, |object| {
        object["profiles"].as_object_mut().unwrap().insert(
            "en-80x24".to_owned(),
            json!("/tmp/skit-ui-walker-v1/sandboxes/other"),
        );
    });
    assert!(SandboxMetadata::decode_and_validate(&wrong_value, &expected).is_err());
    let wrong_value: SandboxMetadata = serde_json::from_slice(&wrong_value).unwrap();
    assert!(crate::sandbox::validate_sandbox_metadata(&wrong_value).is_err());

    let macos_stable = canonical_edit(&bytes, |object| {
        object.insert("platform".to_owned(), json!("macos"));
        object.insert("root".to_owned(), json!("/private/tmp/skit-ui-walker-v1"));
        object["profiles"].as_object_mut().unwrap().insert(
            "en-80x24".to_owned(),
            json!("/private/tmp/skit-ui-walker-v1/sandboxes/en-80x24"),
        );
    });
    let macos_stable: SandboxMetadata = serde_json::from_slice(&macos_stable).unwrap();
    assert!(crate::sandbox::validate_sandbox_metadata(&macos_stable).is_err());

    let random = SandboxMetadata::random(
        SandboxPlatform::Linux,
        "/tmp/random-sandbox",
        profile("en-80x24"),
    )
    .unwrap();
    let random_bytes = random.canonical_bytes().unwrap();
    let wrong_random_root = canonical_edit(&random_bytes, |object| {
        object["profiles"]
            .as_object_mut()
            .unwrap()
            .insert("en-80x24".to_owned(), json!("/tmp/other"));
    });
    let wrong_random_root: SandboxMetadata = serde_json::from_slice(&wrong_random_root).unwrap();
    assert!(crate::sandbox::validate_sandbox_metadata(&wrong_random_root).is_err());
    let multiple_random_profiles = canonical_edit(&random_bytes, |object| {
        object["profiles"]
            .as_object_mut()
            .unwrap()
            .insert("zh-cn-120x30".to_owned(), json!("/tmp/random-sandbox"));
    });
    let multiple_random_profiles: SandboxMetadata =
        serde_json::from_slice(&multiple_random_profiles).unwrap();
    assert!(crate::sandbox::validate_sandbox_metadata(&multiple_random_profiles).is_err());
}

#[test]
fn run_metadata_validation_refuses_schema_one_and_every_field_mismatch() {
    let valid = run_metadata();
    validate_run_metadata(&valid).unwrap();

    let mut corrupt = valid.clone();
    corrupt.schema = 1;
    assert!(validate_run_metadata(&corrupt).is_err());
    let valid_bytes = run_metadata_bytes(&valid).unwrap();
    let schema_one = canonical_edit(&valid_bytes, |object| {
        object.insert("schema".to_owned(), json!(1));
    });
    assert!(decode_run_metadata(&schema_one).is_err());
    let mut corrupt = valid.clone();
    corrupt.source_revision = "  ".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.source_revision = " revision ".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.deterministic_result = "failed".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.operation_count = 0;
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.operations_sha256 = "invalid".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.final_liveness_sha256 = "invalid".to_owned();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.profiles.clear();
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.profiles.push(profile("en-80x24"));
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.profiles = vec![profile("zh-cn-120x30"), profile("en-80x24")];
    assert!(validate_run_metadata(&corrupt).is_err());
    let mut corrupt = valid.clone();
    corrupt.profiles = vec![profile("zh-cn-120x30")];
    assert!(validate_run_metadata(&corrupt).is_err());

    let bytes = run_metadata_bytes(&valid).unwrap();
    assert!(decode_run_metadata(b"[]").is_err());
    let unknown = canonical_edit(&bytes, |object| {
        object.insert("unknown".to_owned(), json!(true));
    });
    assert!(decode_run_metadata(&unknown).is_err());
    for missing in [
        "schema",
        "source_revision",
        "deterministic_result",
        "operation_count",
        "operations_sha256",
        "final_liveness_sha256",
        "profiles",
        "sandbox",
    ] {
        let partial = canonical_edit(&bytes, |object| {
            object.remove(missing);
        });
        assert!(
            decode_run_metadata(&partial).is_err(),
            "accepted missing {missing}"
        );
    }
    let mut noncanonical = vec![b' '];
    noncanonical.extend_from_slice(&bytes);
    assert!(decode_run_metadata(&noncanonical).is_err());
}

#[test]
fn bundle_digest_binds_complete_sandbox_metadata_and_is_order_stable() {
    let first = profile("en-80x24");
    let second = profile("zh-cn-120x30");
    let left_sandbox = sandbox_metadata(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        &[second.as_str(), first.as_str()],
    );
    let right_sandbox = sandbox_metadata(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        &[first.as_str(), second.as_str()],
    );
    let left = BTreeMap::from([
        (second.clone(), "b".repeat(64)),
        (first.clone(), "a".repeat(64)),
    ]);
    let right = BTreeMap::from([
        (first.clone(), "a".repeat(64)),
        (second.clone(), "b".repeat(64)),
    ]);
    let baseline = bundle_digest(&left, "revision", &left_sandbox).unwrap();
    assert_eq!(
        baseline,
        bundle_digest(&right, "revision", &right_sandbox).unwrap()
    );

    let changed_mode = sandbox_metadata_with_mode(
        SandboxMode::Random,
        SandboxPlatform::Linux,
        "/tmp/random-sandbox",
        &[first.as_str()],
    );
    let single_digest = BTreeMap::from([(first.clone(), "a".repeat(64))]);
    let stable_single = sandbox_metadata(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        &[first.as_str()],
    );
    assert_ne!(
        bundle_digest(&single_digest, "revision", &stable_single).unwrap(),
        bundle_digest(&single_digest, "revision", &changed_mode).unwrap()
    );

    let changed_platform = sandbox_metadata(
        SandboxPlatform::Windows,
        r"C:\Temp\skit-ui-walker-v1",
        &[first.as_str(), second.as_str()],
    );
    assert_ne!(
        baseline,
        bundle_digest(&right, "revision", &changed_platform).unwrap()
    );
    let changed_root = sandbox_metadata(
        SandboxPlatform::Linux,
        "/var/tmp/skit-ui-walker-v1",
        &[first.as_str(), second.as_str()],
    );
    assert_ne!(
        baseline,
        bundle_digest(&right, "revision", &changed_root).unwrap()
    );
    let mut changed_value = serde_json::to_value(&right_sandbox).unwrap();
    changed_value["profiles"][first.as_str()] =
        json!("/tmp/skit-ui-walker-v1/sandboxes/en-80x24-other");
    let changed_value: SandboxMetadata = serde_json::from_value(changed_value).unwrap();
    assert!(bundle_digest(&right, "revision", &changed_value).is_err());

    let third = profile("pseudo-120x12");
    let changed_key_sandbox = sandbox_metadata(
        SandboxPlatform::Linux,
        "/tmp/skit-ui-walker-v1",
        &[first.as_str(), third.as_str()],
    );
    let changed_key = BTreeMap::from([(first, "a".repeat(64)), (third, "b".repeat(64))]);
    assert_ne!(
        baseline,
        bundle_digest(&changed_key, "revision", &changed_key_sandbox).unwrap()
    );
    let changed_digest = BTreeMap::from([
        (profile("en-80x24"), "a".repeat(64)),
        (profile("zh-cn-120x30"), "c".repeat(64)),
    ]);
    assert_ne!(
        baseline,
        bundle_digest(&changed_digest, "revision", &right_sandbox).unwrap()
    );
    assert_ne!(
        baseline,
        bundle_digest(&right, "other", &right_sandbox).unwrap()
    );
    assert!(bundle_digest(&right, "", &right_sandbox).is_err());
    assert!(bundle_digest(&right, " revision ", &right_sandbox).is_err());
    assert!(bundle_digest(&BTreeMap::new(), "revision", &right_sandbox).is_err());
    let invalid_digest = BTreeMap::from([
        (profile("en-80x24"), "invalid".to_owned()),
        (profile("zh-cn-120x30"), "b".repeat(64)),
    ]);
    assert!(bundle_digest(&invalid_digest, "revision", &right_sandbox).is_err());
    assert!(
        bundle_digest(&right, "revision", &changed_key_sandbox).is_err(),
        "profile digest keys must equal the sandbox profile keys"
    );

    for (field, wrong) in [("schema", json!(2)), ("namespace", json!("other"))] {
        let mut value = serde_json::to_value(&right_sandbox).unwrap();
        value
            .as_object_mut()
            .unwrap()
            .insert(field.to_owned(), wrong);
        let invalid: SandboxMetadata = serde_json::from_value(value).unwrap();
        assert!(bundle_digest(&right, "revision", &invalid).is_err());
    }
}

#[test]
fn safe_profile_path_apis_never_accept_raw_profile_text() {
    let profile = profile("en-80x24");
    assert_eq!(
        profile_directory(&profile),
        std::path::PathBuf::from("profiles/en-80x24")
    );
    assert_eq!(
        chunk_json_path(&profile, "en-80x24-0000"),
        std::path::PathBuf::from("profiles/en-80x24/chunks/en-80x24-0000.json")
    );
    assert_eq!(
        chunk_view_path(&profile, "en-80x24-0000"),
        std::path::PathBuf::from("profiles/en-80x24/chunks/en-80x24-0000.md")
    );
    assert_eq!(
        profile_lease_path(&profile),
        std::path::PathBuf::from("leases/en-80x24.lock")
    );
    assert_eq!(
        profile_sandbox_path(&profile),
        std::path::PathBuf::from("sandboxes/en-80x24")
    );
    assert_eq!(
        profile_cleanup_path(&profile),
        std::path::PathBuf::from("sandboxes/.cleanup-en-80x24")
    );
}
