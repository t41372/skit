use skit_application::form_feedback::{GlobCountPort, GlobCountRequest, glob_count_request};

#[derive(Debug)]
struct Counter;

impl GlobCountPort for Counter {
    fn count_matches(&self, request: &GlobCountRequest) -> usize {
        request
            .pieces
            .iter()
            .map(|piece| usize::from(piece.contains('*')) * 2 + usize::from(!piece.contains('*')))
            .sum()
    }
}

#[test]
fn glob_feedback_requests_are_typed_split_and_adapter_driven() {
    assert_eq!(glob_count_request("plain.txt", "/work"), None);
    assert_eq!(glob_count_request("'unfinished", "/work"), None);

    let request = glob_count_request("src/*.rs README.md", "/work").unwrap();
    assert_eq!(request.cwd, "/work");
    assert_eq!(request.pieces, ["src/*.rs", "README.md"]);
    assert_eq!(Counter.count_matches(&request), 3);
}
