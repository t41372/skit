use std::{fs,path::Path};
#[test]
fn store_parity_requires_exact_fault_contracts(){
 let repo=Path::new(env!("CARGO_MANIFEST_DIR")).parent().and_then(Path::parent).unwrap();
 let source=fs::read_to_string(repo.join("crates/skit-store/tests/port_test_store_index_edges.rs")).unwrap();
 assert!(!source.contains("fn test_a_store_that_cannot_be_written_still_lists"));
}
