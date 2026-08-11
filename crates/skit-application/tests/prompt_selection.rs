use std::sync::Mutex;

use skit_application::{
    form_state::StateWriteError,
    prompt_selection::{PromptSelectionService, PromptSelectionStore},
};

#[derive(Debug, Default)]
struct MemoryPromptSelectionStore {
    runner: Mutex<String>,
}

impl PromptSelectionStore for MemoryPromptSelectionStore {
    fn load_last_runner(&self) -> String {
        self.runner.lock().unwrap().clone()
    }

    fn save_last_runner(&self, name: &str) -> Result<(), StateWriteError> {
        *self.runner.lock().unwrap() = name.to_owned();
        Ok(())
    }
}

#[test]
fn service_keeps_last_runner_state_separate_from_runner_resolution() {
    let service = PromptSelectionService::new(MemoryPromptSelectionStore::default());
    assert_eq!(service.last_runner(), "");

    service.remember_runner("codex").unwrap();
    assert_eq!(service.last_runner(), "codex");

    service.remember_runner("").unwrap();
    assert_eq!(service.last_runner(), "");
}
