//! Shared frontend-neutral picker and file-browser state.

use std::{
    collections::BTreeSet,
    path::{Path, PathBuf},
};

use nucleo_matcher::{
    Config as MatcherConfig, Matcher, Utf32Str,
    pattern::{AtomKind, CaseMatching, Normalization, Pattern},
};

/// Single- or multiple-selection behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerMode {
    /// Selecting one item accepts it.
    Single,
    /// Space and mouse clicks build an explicit set before acceptance.
    Multiple,
}

/// One typed item and its searchable user or registry text.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PickerItem<T> {
    /// Stable typed identity.
    pub id: T,
    /// Search corpus. Renderers can localize a registry identity separately.
    pub search_text: String,
}

impl<T> PickerItem<T> {
    /// Create one item.
    #[must_use]
    pub fn new(id: T, search_text: impl Into<String>) -> Self {
        Self {
            id,
            search_text: search_text.into(),
        }
    }
}

/// Result returned to the owning workflow.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PickerResult<T> {
    /// The picker was cancelled without applying its working set.
    Cancelled,
    /// One value from a single picker.
    One(T),
    /// Complete accepted set from a multiple picker.
    Many(Vec<T>),
}

/// Frontend-neutral filtered choices. Ratatui's mature list state owns cursor and scrolling.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChoicePicker<T>
where
    T: Clone + Eq + Ord,
{
    mode: PickerMode,
    items: Vec<PickerItem<T>>,
    visible: Vec<usize>,
    selected: BTreeSet<T>,
    query: String,
}

impl<T> ChoicePicker<T>
where
    T: Clone + Eq + Ord,
{
    /// Create a filtered choice model and an isolated working selection.
    #[must_use]
    pub fn new(mode: PickerMode, items: Vec<PickerItem<T>>, selected: Vec<T>) -> Self {
        let mut picker = Self {
            mode,
            visible: (0..items.len()).collect(),
            items,
            selected: selected.into_iter().collect(),
            query: String::new(),
        };
        picker.recompute();
        picker
    }

    /// Selection behavior.
    #[must_use]
    pub const fn mode(&self) -> PickerMode {
        self.mode
    }

    /// Current search query.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replace the search query and recompute with the mature fuzzy matcher.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
        self.recompute();
    }

    /// Items in visible rank order.
    #[must_use]
    pub fn visible_items(&self) -> Vec<&PickerItem<T>> {
        self.visible
            .iter()
            .filter_map(|index| self.items.get(*index))
            .collect()
    }

    /// Return whether one item is in the working selection.
    #[must_use]
    pub fn is_selected(&self, id: &T) -> bool {
        self.selected.contains(id)
    }

    /// Toggle one item in a multiple picker.
    pub fn toggle(&mut self, id: &T) {
        if self.mode != PickerMode::Multiple || !self.items.iter().any(|item| &item.id == id) {
            return;
        }
        if !self.selected.remove(id) {
            self.selected.insert(id.clone());
        }
    }

    /// Select or clear every currently visible item.
    pub fn select_visible(&mut self, selected: bool) {
        if self.mode != PickerMode::Multiple {
            return;
        }
        for index in &self.visible {
            let id = self.items[*index].id.clone();
            if selected {
                self.selected.insert(id);
            } else {
                self.selected.remove(&id);
            }
        }
    }

    /// Select or clear the complete item set, independent of the active filter.
    pub fn select_all(&mut self, selected: bool) {
        if self.mode != PickerMode::Multiple {
            return;
        }
        if selected {
            self.selected = self.items.iter().map(|item| item.id.clone()).collect();
        } else {
            self.selected.clear();
        }
    }

    /// Return whether every item is selected. An empty picker is not selected-all.
    #[must_use]
    pub fn all_selected(&self) -> bool {
        !self.items.is_empty()
            && self
                .items
                .iter()
                .all(|item| self.selected.contains(&item.id))
    }

    /// Accept one cursor item from a mature single-picker session.
    #[must_use]
    pub fn accept_item(&self, id: &T) -> Option<PickerResult<T>> {
        (self.mode == PickerMode::Single && self.items.iter().any(|item| &item.id == id))
            .then(|| PickerResult::One(id.clone()))
    }

    /// Accept a multiple working set in original item order.
    #[must_use]
    pub fn accept(&self) -> PickerResult<T> {
        match self.mode {
            PickerMode::Single => self
                .visible
                .first()
                .and_then(|index| self.items.get(*index))
                .map(|item| PickerResult::One(item.id.clone()))
                .unwrap_or(PickerResult::Cancelled),
            PickerMode::Multiple => PickerResult::Many(
                self.items
                    .iter()
                    .filter(|item| self.selected.contains(&item.id))
                    .map(|item| item.id.clone())
                    .collect(),
            ),
        }
    }

    /// Cancel without publishing the working set.
    #[must_use]
    pub const fn cancel(&self) -> PickerResult<T> {
        PickerResult::Cancelled
    }

    fn recompute(&mut self) {
        if self.query.is_empty() {
            self.visible = (0..self.items.len()).collect();
            return;
        }
        let pattern = Pattern::new(
            &self.query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let query_lower = self.query.to_lowercase();
        let mut matcher = Matcher::new(MatcherConfig::DEFAULT);
        let mut utf32 = Vec::new();
        let mut ranked = self
            .items
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let score =
                    pattern.score(Utf32Str::new(&item.search_text, &mut utf32), &mut matcher)?;
                let prefix = item.search_text.to_lowercase().starts_with(&query_lower);
                Some((index, prefix, score))
            })
            .collect::<Vec<_>>();
        ranked.sort_by(|left, right| {
            right
                .1
                .cmp(&left.1)
                .then_with(|| {
                    self.items[left.0]
                        .search_text
                        .chars()
                        .count()
                        .cmp(&self.items[right.0].search_text.chars().count())
                })
                .then_with(|| right.2.cmp(&left.2))
                .then_with(|| left.0.cmp(&right.0))
        });
        self.visible = ranked.into_iter().map(|(index, _, _)| index).collect();
    }
}

/// Product operation that requested a filesystem picker.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PickerPurpose {
    /// Add-source file or directory.
    Source,
    /// A path-valued run-form field.
    Argument,
    /// A working-directory setting.
    WorkingDirectory,
    /// An editor executable or other configuration file.
    Configuration,
}

/// Filesystem object kinds that can be accepted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PathSelectionMode {
    /// Regular file only.
    File,
    /// Directory only.
    Directory,
    /// File or directory.
    FileOrDirectory,
}

/// How an accepted absolute path is returned to the form.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PathOutputPolicy {
    /// Preserve the absolute picker result.
    Absolute,
    /// Use a relative path when the result is below this base.
    RelativeTo(PathBuf),
}

/// Frontend-neutral filesystem-picker contract.
///
/// `ratatui-interact::FileExplorerState` owns directory reads, cursor, and scroll in the TUI
/// adapter. This type owns only product semantics and accepted output.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PathPickerState {
    purpose: PickerPurpose,
    start_dir: PathBuf,
    selection: PathSelectionMode,
    output: PathOutputPolicy,
    allow_multiple: bool,
    show_hidden: bool,
    query: String,
}

impl PathPickerState {
    /// Create one typed filesystem request.
    #[must_use]
    pub fn new(
        purpose: PickerPurpose,
        start_dir: PathBuf,
        selection: PathSelectionMode,
        output: PathOutputPolicy,
        allow_multiple: bool,
    ) -> Self {
        Self {
            purpose,
            start_dir,
            selection,
            output,
            allow_multiple,
            show_hidden: false,
            query: String::new(),
        }
    }

    /// Owning operation.
    #[must_use]
    pub const fn purpose(&self) -> PickerPurpose {
        self.purpose
    }

    /// Initial directory selected by the host's workdir/origin/invoke policy.
    #[must_use]
    pub fn start_dir(&self) -> &Path {
        &self.start_dir
    }

    /// Accepted filesystem object shape.
    #[must_use]
    pub const fn selection(&self) -> PathSelectionMode {
        self.selection
    }

    /// Whether more than one path can be returned.
    #[must_use]
    pub const fn allow_multiple(&self) -> bool {
        self.allow_multiple
    }

    /// Whether dotfiles are visible.
    #[must_use]
    pub const fn show_hidden(&self) -> bool {
        self.show_hidden
    }

    /// Toggle dotfile visibility without changing the search query.
    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
    }

    /// Current file filter.
    #[must_use]
    pub fn query(&self) -> &str {
        &self.query
    }

    /// Replace the file filter.
    pub fn set_query(&mut self, query: impl Into<String>) {
        self.query = query.into();
    }

    /// Apply the output policy after a mature explorer accepts one absolute path.
    #[must_use]
    pub fn output_path(&self, path: &Path) -> PathBuf {
        match &self.output {
            PathOutputPolicy::Absolute => path.to_path_buf(),
            PathOutputPolicy::RelativeTo(base) => path
                .strip_prefix(base)
                .map_or_else(|_| path.to_path_buf(), PathBuf::from),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
    enum Choice {
        Alpha,
        Alphabet,
        Graph,
    }

    #[test]
    fn search_is_case_insensitive_and_prefix_matches_rank_before_substrings() {
        let mut picker = ChoicePicker::new(
            PickerMode::Single,
            vec![
                PickerItem::new(Choice::Graph, "photograph"),
                PickerItem::new(Choice::Alphabet, "Alphabet"),
                PickerItem::new(Choice::Alpha, "alpha"),
            ],
            Vec::new(),
        );
        picker.set_query("ALP");

        assert_eq!(
            picker
                .visible_items()
                .iter()
                .map(|item| &item.id)
                .collect::<Vec<_>>(),
            vec![&Choice::Alpha, &Choice::Alphabet]
        );
    }

    #[test]
    fn a_multiple_picker_changes_nothing_until_accept() {
        let mut picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![
                PickerItem::new(Choice::Alpha, "alpha"),
                PickerItem::new(Choice::Graph, "graph"),
            ],
            vec![Choice::Alpha],
        );
        picker.toggle(&Choice::Alpha);
        picker.toggle(&Choice::Graph);

        assert_eq!(picker.cancel(), PickerResult::Cancelled);
        assert_eq!(picker.accept(), PickerResult::Many(vec![Choice::Graph]));
    }

    #[test]
    fn file_output_policy_keeps_path_semantics_out_of_renderers() {
        let picker = PathPickerState::new(
            PickerPurpose::Source,
            PathBuf::from("/work/project"),
            PathSelectionMode::FileOrDirectory,
            PathOutputPolicy::RelativeTo(PathBuf::from("/work")),
            false,
        );

        assert_eq!(
            picker.output_path(&PathBuf::from("/work/project/tool.py")),
            PathBuf::from("project/tool.py")
        );
        assert_eq!(
            picker.output_path(&PathBuf::from("/elsewhere/tool.py")),
            PathBuf::from("/elsewhere/tool.py")
        );
    }

    #[test]
    fn hidden_and_filter_controls_are_typed_and_independent() {
        let mut picker = PathPickerState::new(
            PickerPurpose::Argument,
            PathBuf::from("/work"),
            PathSelectionMode::File,
            PathOutputPolicy::Absolute,
            true,
        );
        picker.set_query("report");
        picker.toggle_hidden();

        assert_eq!(picker.query(), "report");
        assert!(picker.show_hidden());
        assert!(picker.allow_multiple());
        assert_eq!(picker.purpose(), PickerPurpose::Argument);
    }

    #[test]
    fn select_all_applies_to_the_complete_choice_set_while_filtered() {
        let mut picker = ChoicePicker::new(
            PickerMode::Multiple,
            vec![
                PickerItem::new(Choice::Alpha, "alpha"),
                PickerItem::new(Choice::Alphabet, "alphabet"),
                PickerItem::new(Choice::Graph, "graph"),
            ],
            vec![Choice::Graph],
        );
        picker.set_query("alpha");

        picker.select_all(true);
        assert!(picker.all_selected());
        assert_eq!(
            picker.accept(),
            PickerResult::Many(vec![Choice::Alpha, Choice::Alphabet, Choice::Graph])
        );

        picker.select_all(false);
        assert!(!picker.all_selected());
        assert_eq!(picker.accept(), PickerResult::Many(Vec::new()));
    }

    #[test]
    fn single_and_multiple_picker_operations_keep_their_typed_boundaries() {
        let items = vec![
            PickerItem::new(Choice::Alpha, "alpha"),
            PickerItem::new(Choice::Alphabet, "alphabet"),
            PickerItem::new(Choice::Graph, "graph"),
        ];
        let mut single = ChoicePicker::new(PickerMode::Single, items.clone(), Vec::new());
        assert_eq!(single.mode(), PickerMode::Single);
        assert_eq!(single.query(), "");
        assert_eq!(
            single.accept_item(&Choice::Graph),
            Some(PickerResult::One(Choice::Graph))
        );
        assert_eq!(
            single.accept_item(&Choice::Alphabet),
            Some(PickerResult::One(Choice::Alphabet))
        );
        single.toggle(&Choice::Alpha);
        single.select_visible(true);
        single.select_all(true);
        assert!(!single.is_selected(&Choice::Alpha));
        assert_eq!(single.accept(), PickerResult::One(Choice::Alpha));

        single.set_query("missing");
        assert_eq!(single.query(), "missing");
        assert_eq!(single.accept(), PickerResult::Cancelled);
        assert_eq!(
            single.accept_item(&Choice::Graph),
            Some(PickerResult::One(Choice::Graph))
        );

        let empty = ChoicePicker::<Choice>::new(PickerMode::Single, Vec::new(), Vec::new());
        assert_eq!(empty.accept(), PickerResult::Cancelled);
        assert_eq!(empty.accept_item(&Choice::Graph), None);

        let mut multiple = ChoicePicker::new(PickerMode::Multiple, items, Vec::new());
        assert_eq!(multiple.mode(), PickerMode::Multiple);
        assert_eq!(multiple.accept_item(&Choice::Alpha), None);
        multiple.toggle(&Choice::Alpha);
        multiple.toggle(&Choice::Alpha);
        multiple.toggle(&Choice::Graph);
        multiple.set_query("alpha");
        multiple.select_visible(true);
        assert!(multiple.is_selected(&Choice::Alpha));
        assert!(multiple.is_selected(&Choice::Alphabet));
        assert!(multiple.is_selected(&Choice::Graph));
        multiple.select_visible(false);
        assert!(!multiple.is_selected(&Choice::Alpha));
        assert!(!multiple.is_selected(&Choice::Alphabet));
        assert!(multiple.is_selected(&Choice::Graph));
        multiple.toggle(&Choice::Graph);
        assert_eq!(multiple.accept(), PickerResult::Many(Vec::new()));
    }
}
