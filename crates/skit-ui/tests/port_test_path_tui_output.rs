use skit_ui::{PathOutputPolicy, PathPickerState, PathSelectionMode, PickerPurpose};

#[test]
fn test_value_for_is_relative_inside_the_root_and_posix_everywhere() {
    let temp = tempfile::tempdir().expect("tempdir");
    let root = temp.path().join("root");
    std::fs::create_dir_all(root.join("sub")).expect("subdir");
    let inner = root.join("sub/inner.txt");
    std::fs::write(&inner, b"x").expect("inner");
    let outside = temp.path().join("other.txt");
    std::fs::write(&outside, b"x").expect("outside");

    let picker = PathPickerState::new(
        PickerPurpose::Argument,
        root.clone(),
        PathSelectionMode::FileOrDirectory,
        PathOutputPolicy::RelativeTo(root.clone()),
        false,
    );

    assert_eq!(
        picker.output_path(&inner).to_string_lossy(),
        "sub/inner.txt",
        "relative picker output is POSIX text on every platform"
    );
    assert_eq!(
        picker.output_path(&root).to_string_lossy(),
        ".",
        "selecting the completion root itself is represented by a single dot"
    );
    let expected_outside = outside.to_string_lossy().replace('\\', "/");
    assert_eq!(
        picker.output_path(&outside).to_string_lossy(),
        expected_outside,
        "absolute fallback output is also POSIX text on every platform"
    );
}
