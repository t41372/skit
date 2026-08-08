use std::path::PathBuf;

use skit_core::{PathContext, Platform, resolve_roots};

fn path(value: &str) -> Option<PathBuf> {
    Some(PathBuf::from(value))
}

#[test]
fn linux_matches_the_existing_platformdirs_layout() -> Result<(), Box<dyn std::error::Error>> {
    let context = PathContext {
        home: path("/home/alice"),
        ..PathContext::default()
    };
    let roots = resolve_roots(Platform::Linux, &context)?;
    assert_eq!(
        roots.data_dir(),
        PathBuf::from("/home/alice/.local/share/skit")
    );
    assert_eq!(
        roots.state_dir(),
        PathBuf::from("/home/alice/.local/state/skit")
    );
    assert_eq!(
        roots.config_dir(),
        PathBuf::from("/home/alice/.config/skit")
    );
    Ok(())
}

#[test]
fn linux_honors_xdg_roots_before_adding_the_app_name() -> Result<(), Box<dyn std::error::Error>> {
    let context = PathContext {
        home: path("/home/alice"),
        xdg_data_home: path("/data"),
        xdg_state_home: path("/state"),
        xdg_config_home: path("/config"),
        ..PathContext::default()
    };
    let roots = resolve_roots(Platform::Linux, &context)?;
    assert_eq!(roots.data_dir(), PathBuf::from("/data/skit"));
    assert_eq!(roots.state_dir(), PathBuf::from("/state/skit"));
    assert_eq!(roots.config_dir(), PathBuf::from("/config/skit"));
    Ok(())
}

#[test]
fn macos_uses_one_application_support_root() -> Result<(), Box<dyn std::error::Error>> {
    let context = PathContext {
        home: path("/Users/alice"),
        ..PathContext::default()
    };
    let roots = resolve_roots(Platform::MacOs, &context)?;
    let expected = PathBuf::from("/Users/alice/Library/Application Support/skit");
    assert_eq!(roots.data_dir(), expected);
    assert_eq!(roots.state_dir(), roots.data_dir());
    assert_eq!(roots.config_dir(), roots.data_dir());
    Ok(())
}

#[test]
fn windows_uses_local_app_data_for_all_layers() -> Result<(), Box<dyn std::error::Error>> {
    let context = PathContext {
        local_app_data: path(r"C:\Users\Alice\AppData\Local"),
        ..PathContext::default()
    };
    let roots = resolve_roots(Platform::Windows, &context)?;
    let expected = PathBuf::from(r"C:\Users\Alice\AppData\Local").join("skit");
    assert_eq!(roots.data_dir(), expected);
    assert_eq!(roots.state_dir(), roots.data_dir());
    assert_eq!(roots.config_dir(), roots.data_dir());
    Ok(())
}

#[test]
fn skit_overrides_are_per_axis_and_do_not_get_an_extra_app_suffix()
-> Result<(), Box<dyn std::error::Error>> {
    let context = PathContext {
        home: path("/home/alice"),
        data_override: path("/custom/data"),
        config_override: path("/custom/config"),
        ..PathContext::default()
    };
    let roots = resolve_roots(Platform::Linux, &context)?;
    assert_eq!(roots.data_dir(), PathBuf::from("/custom/data"));
    assert_eq!(
        roots.state_dir(),
        PathBuf::from("/home/alice/.local/state/skit")
    );
    assert_eq!(roots.config_dir(), PathBuf::from("/custom/config"));
    Ok(())
}

#[test]
fn three_overrides_need_no_home_or_platform_directory() -> Result<(), Box<dyn std::error::Error>> {
    let context = PathContext {
        data_override: path("data"),
        state_override: path("state"),
        config_override: path("config"),
        ..PathContext::default()
    };
    for platform in [Platform::Linux, Platform::MacOs, Platform::Windows] {
        let roots = resolve_roots(platform, &context)?;
        assert_eq!(roots.data_dir(), PathBuf::from("data"));
        assert_eq!(roots.state_dir(), PathBuf::from("state"));
        assert_eq!(roots.config_dir(), PathBuf::from("config"));
    }
    Ok(())
}
