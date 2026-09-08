//! Deterministic artifact schemas and validators for the skit TUI model walker.
//!
//! This crate has no product behavior. It gives the local walker stable content objects,
//! styled terminal frames, transition timelines, review chunks, manifests, and progress checks.

#![forbid(unsafe_code)]
#![deny(missing_docs)]

/// Deterministic asciicast v3 recording for complete styled walker frames.
pub mod asciicast;
/// Deterministic paths, bytes, metadata, and layouts for installed bundles.
pub mod bundle;
/// Generic deterministic walker orchestration over frontend and host adapters.
pub mod engine;
/// Exact JSON, text, and draft-name scans for artifact leak oracles.
pub mod leak_oracle;
/// Product-independent JSON and text projection helpers.
pub mod projection;
/// Sandbox identities, ownership markers, and cleanup decisions.
pub mod sandbox;
/// Type-compatible deterministic sentinels for volatile host values.
pub mod sentinels;

#[cfg(test)]
mod bundle_tests;

#[cfg(test)]
mod coverage_tests;

#[cfg(test)]
mod aggregate_contract_tests;

#[cfg(test)]
mod engine_tests;

#[cfg(test)]
mod sandbox_tests;

use std::{collections::BTreeMap, error::Error, fmt};

use ratatui_core::{
    backend::TestBackend,
    buffer::{Buffer, CellDiffOption, CellWidth},
    layout::{Position, Rect},
    style::{Color, Modifier},
};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

use sandbox::SafeProfileId;
pub use sandbox::validate_stable_namespace_root;

const OBJECT_SCHEMA: u16 = 1;

#[derive(Clone, Debug, Eq, PartialEq)]
/// An invalid or inconsistent walker artifact.
pub struct ArtifactError(String);

impl ArtifactError {
    fn new(message: impl Into<String>) -> Self {
        Self(message.into())
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl Error for ArtifactError {}

impl From<serde_json::Error> for ArtifactError {
    fn from(error: serde_json::Error) -> Self {
        Self::new(error.to_string())
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "snake_case")]
/// The complete state object referenced by one timeline row.
pub enum ObjectKind {
    /// Frontend-neutral reducer state.
    Reducer,
    /// Deterministic host state.
    Host,
    /// Persistent TUI session state.
    Session,
    /// Styled terminal buffer and cursor state.
    StyledFrame,
    /// Rendered mouse and layout geometry.
    Geometry,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// A typed SHA-256 reference to one canonical content object.
pub struct ObjectRef {
    /// The object payload kind.
    pub kind: ObjectKind,
    /// The lowercase SHA-256 digest of the canonical object envelope.
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct ContentObject {
    schema: u16,
    kind: ObjectKind,
    value: Value,
}

/// Serialize one JSON value with recursively sorted object keys.
pub fn canonical_json_bytes(value: &Value) -> Result<Vec<u8>, ArtifactError> {
    serde_json::to_vec(&canonical_value(value)).map_err(ArtifactError::from)
}

fn canonical_value(value: &Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let mut keys = values.keys().collect::<Vec<_>>();
            keys.sort_unstable();
            let mut canonical = serde_json::Map::new();
            for key in keys {
                canonical.insert(key.clone(), canonical_value(&values[key]));
            }
            Value::Object(canonical)
        }
        scalar => scalar.clone(),
    }
}

/// Build the canonical content object and its typed reference.
pub fn build_object(
    kind: ObjectKind,
    value: &Value,
) -> Result<(ObjectRef, Vec<u8>), ArtifactError> {
    let envelope = ContentObject {
        schema: OBJECT_SCHEMA,
        kind,
        value: value.clone(),
    };
    let bytes = canonical_json_bytes(&serde_json::to_value(envelope)?)?;
    let reference = ObjectRef {
        kind,
        sha256: sha256_hex(&bytes),
    };
    Ok((reference, bytes))
}

/// Compute the typed reference for one content object.
pub fn object_ref(kind: ObjectKind, value: &Value) -> Result<ObjectRef, ArtifactError> {
    build_object(kind, value).map(|(reference, _)| reference)
}

/// Validate canonical bytes against a typed content reference.
pub fn validate_object(reference: &ObjectRef, bytes: &[u8]) -> Result<Value, ArtifactError> {
    validate_digest(&reference.sha256)?;
    let decoded: Value = serde_json::from_slice(bytes)?;
    let canonical = canonical_json_bytes(&decoded)?;
    if bytes != canonical {
        return Err(ArtifactError::new("object bytes are not canonical JSON"));
    }
    let object: ContentObject = serde_json::from_value(decoded)?;
    if object.schema != OBJECT_SCHEMA {
        return Err(ArtifactError::new(format!(
            "unsupported object schema: {}",
            object.schema
        )));
    }
    if object.kind != reference.kind {
        return Err(ArtifactError::new(
            "object kind does not match its reference",
        ));
    }
    let actual = sha256_hex(bytes);
    if actual != reference.sha256 {
        return Err(ArtifactError::new(format!(
            "object digest mismatch: expected {}, got {actual}",
            reference.sha256
        )));
    }
    let typed = canonical_json_bytes(&serde_json::to_value(&object)?)?;
    if bytes != typed {
        return Err(ArtifactError::new(
            "typed object does not match its stored bytes",
        ));
    }
    Ok(object.value)
}

fn validate_digest(digest: &str) -> Result<(), ArtifactError> {
    if digest.len() != 64
        || !digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(ArtifactError::new(format!(
            "invalid lowercase SHA-256 digest: {digest}"
        )));
    }
    Ok(())
}

fn require(condition: bool, message: &'static str) -> Result<(), ArtifactError> {
    condition
        .then_some(())
        .ok_or_else(|| ArtifactError::new(message))
}

/// Compute a lowercase SHA-256 digest.
pub fn sha256_hex(bytes: &[u8]) -> String {
    let mut output = String::with_capacity(64);
    for byte in Sha256::digest(bytes) {
        use fmt::Write as _;
        write!(output, "{byte:02x}").expect("writing to a String cannot fail");
    }
    output
}

/// Validate the JSON-line structure of one asciicast v3 file.
pub fn validate_asciicast_v3(bytes: &[u8]) -> Result<(), ArtifactError> {
    let mut lines = bytes.split(|byte| *byte == b'\n');
    let header_bytes = lines
        .next()
        .filter(|line| !line.is_empty())
        .ok_or_else(|| ArtifactError::new("asciicast has no header"))?;
    let header: Value = serde_json::from_slice(header_bytes)?;
    let term = header
        .as_object()
        .filter(|header| header.get("version").and_then(Value::as_u64) == Some(3))
        .and_then(|header| header.get("term"))
        .and_then(Value::as_object)
        .ok_or_else(|| ArtifactError::new("asciicast v3 header is invalid"))?;
    let positive_dimension = |name: &str| {
        term.get(name)
            .and_then(Value::as_u64)
            .is_some_and(|dimension| dimension > 0)
    };
    if !positive_dimension("cols") || !positive_dimension("rows") {
        return Err(ArtifactError::new(
            "asciicast terminal dimensions must be positive",
        ));
    }

    let remaining = lines.collect::<Vec<_>>();
    for (index, line) in remaining.iter().enumerate() {
        if line.is_empty() && index + 1 == remaining.len() {
            continue;
        }
        let event: Value = serde_json::from_slice(line)?;
        let event = event
            .as_array()
            .filter(|event| event.len() == 3)
            .ok_or_else(|| ArtifactError::new("asciicast event must have three fields"))?;
        let delay = event[0]
            .as_f64()
            .filter(|delay| delay.is_finite() && *delay >= 0.0)
            .ok_or_else(|| ArtifactError::new("asciicast event delay is invalid"))?;
        debug_assert!(delay.is_finite());
        if !matches!(event[1].as_str(), Some("o" | "r")) || !event[2].is_string() {
            return Err(ArtifactError::new(
                "asciicast event type or payload is invalid",
            ));
        }
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// A serializable terminal rectangle.
pub struct RectSnapshot {
    /// The left column.
    pub x: u16,
    /// The top row.
    pub y: u16,
    /// The width in terminal cells.
    pub width: u16,
    /// The height in terminal cells.
    pub height: u16,
}

impl From<Rect> for RectSnapshot {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.x,
            y: rect.y,
            width: rect.width,
            height: rect.height,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// A serializable terminal position.
pub struct PositionSnapshot {
    /// The terminal column.
    pub x: u16,
    /// The terminal row.
    pub y: u16,
}

impl From<Position> for PositionSnapshot {
    fn from(position: Position) -> Self {
        Self {
            x: position.x,
            y: position.y,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
/// A stable representation of one Ratatui color.
pub enum ColorSnapshot {
    /// The terminal default color.
    Reset,
    /// ANSI black.
    Black,
    /// ANSI red.
    Red,
    /// ANSI green.
    Green,
    /// ANSI yellow.
    Yellow,
    /// ANSI blue.
    Blue,
    /// ANSI magenta.
    Magenta,
    /// ANSI cyan.
    Cyan,
    /// ANSI gray.
    Gray,
    /// ANSI dark gray.
    DarkGray,
    /// ANSI light red.
    LightRed,
    /// ANSI light green.
    LightGreen,
    /// ANSI light yellow.
    LightYellow,
    /// ANSI light blue.
    LightBlue,
    /// ANSI light magenta.
    LightMagenta,
    /// ANSI light cyan.
    LightCyan,
    /// ANSI white.
    White,
    /// A 24-bit color.
    Rgb {
        /// The red channel.
        red: u8,
        /// The green channel.
        green: u8,
        /// The blue channel.
        blue: u8,
    },
    /// An indexed terminal color.
    Indexed {
        /// The palette index.
        index: u8,
    },
}

impl From<Color> for ColorSnapshot {
    fn from(color: Color) -> Self {
        match color {
            Color::Reset => Self::Reset,
            Color::Black => Self::Black,
            Color::Red => Self::Red,
            Color::Green => Self::Green,
            Color::Yellow => Self::Yellow,
            Color::Blue => Self::Blue,
            Color::Magenta => Self::Magenta,
            Color::Cyan => Self::Cyan,
            Color::Gray => Self::Gray,
            Color::DarkGray => Self::DarkGray,
            Color::LightRed => Self::LightRed,
            Color::LightGreen => Self::LightGreen,
            Color::LightYellow => Self::LightYellow,
            Color::LightBlue => Self::LightBlue,
            Color::LightMagenta => Self::LightMagenta,
            Color::LightCyan => Self::LightCyan,
            Color::White => Self::White,
            Color::Rgb(red, green, blue) => Self::Rgb { red, green, blue },
            Color::Indexed(index) => Self::Indexed { index },
        }
    }
}

impl From<ColorSnapshot> for Color {
    fn from(color: ColorSnapshot) -> Self {
        match color {
            ColorSnapshot::Reset => Self::Reset,
            ColorSnapshot::Black => Self::Black,
            ColorSnapshot::Red => Self::Red,
            ColorSnapshot::Green => Self::Green,
            ColorSnapshot::Yellow => Self::Yellow,
            ColorSnapshot::Blue => Self::Blue,
            ColorSnapshot::Magenta => Self::Magenta,
            ColorSnapshot::Cyan => Self::Cyan,
            ColorSnapshot::Gray => Self::Gray,
            ColorSnapshot::DarkGray => Self::DarkGray,
            ColorSnapshot::LightRed => Self::LightRed,
            ColorSnapshot::LightGreen => Self::LightGreen,
            ColorSnapshot::LightYellow => Self::LightYellow,
            ColorSnapshot::LightBlue => Self::LightBlue,
            ColorSnapshot::LightMagenta => Self::LightMagenta,
            ColorSnapshot::LightCyan => Self::LightCyan,
            ColorSnapshot::White => Self::White,
            ColorSnapshot::Rgb { red, green, blue } => Self::Rgb(red, green, blue),
            ColorSnapshot::Indexed { index } => Self::Indexed(index),
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// One stable text modifier.
pub enum ModifierSnapshot {
    /// Bold text.
    Bold,
    /// Dim text.
    Dim,
    /// Italic text.
    Italic,
    /// Underlined text.
    Underlined,
    /// Slowly blinking text.
    SlowBlink,
    /// Rapidly blinking text.
    RapidBlink,
    /// Reversed foreground and background.
    Reversed,
    /// Hidden text.
    Hidden,
    /// Crossed-out text.
    CrossedOut,
}

fn modifier_snapshot(modifiers: Modifier) -> Vec<ModifierSnapshot> {
    [
        (Modifier::BOLD, ModifierSnapshot::Bold),
        (Modifier::DIM, ModifierSnapshot::Dim),
        (Modifier::ITALIC, ModifierSnapshot::Italic),
        (Modifier::UNDERLINED, ModifierSnapshot::Underlined),
        (Modifier::SLOW_BLINK, ModifierSnapshot::SlowBlink),
        (Modifier::RAPID_BLINK, ModifierSnapshot::RapidBlink),
        (Modifier::REVERSED, ModifierSnapshot::Reversed),
        (Modifier::HIDDEN, ModifierSnapshot::Hidden),
        (Modifier::CROSSED_OUT, ModifierSnapshot::CrossedOut),
    ]
    .into_iter()
    .filter_map(|(modifier, snapshot)| modifiers.contains(modifier).then_some(snapshot))
    .collect()
}

impl From<ModifierSnapshot> for Modifier {
    fn from(modifier: ModifierSnapshot) -> Self {
        match modifier {
            ModifierSnapshot::Bold => Self::BOLD,
            ModifierSnapshot::Dim => Self::DIM,
            ModifierSnapshot::Italic => Self::ITALIC,
            ModifierSnapshot::Underlined => Self::UNDERLINED,
            ModifierSnapshot::SlowBlink => Self::SLOW_BLINK,
            ModifierSnapshot::RapidBlink => Self::RAPID_BLINK,
            ModifierSnapshot::Reversed => Self::REVERSED,
            ModifierSnapshot::Hidden => Self::HIDDEN,
            ModifierSnapshot::CrossedOut => Self::CROSSED_OUT,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
/// A stable Ratatui cell diff option.
pub enum CellDiffSnapshot {
    /// Use normal cell comparison.
    None,
    /// Do not copy the cell during a diff.
    Skip,
    /// Copy the cell for every diff.
    AlwaysUpdate,
    /// Use an explicit display width.
    ForcedWidth {
        /// The nonzero width in terminal cells.
        width: u16,
    },
}

impl From<CellDiffOption> for CellDiffSnapshot {
    fn from(option: CellDiffOption) -> Self {
        match option {
            CellDiffOption::None => Self::None,
            CellDiffOption::Skip => Self::Skip,
            CellDiffOption::AlwaysUpdate => Self::AlwaysUpdate,
            CellDiffOption::ForcedWidth(width) => Self::ForcedWidth { width: width.get() },
        }
    }
}

fn cell_diff_option(snapshot: CellDiffSnapshot) -> Result<CellDiffOption, ArtifactError> {
    match snapshot {
        CellDiffSnapshot::None => Ok(CellDiffOption::None),
        CellDiffSnapshot::Skip => Ok(CellDiffOption::Skip),
        CellDiffSnapshot::AlwaysUpdate => Ok(CellDiffOption::AlwaysUpdate),
        CellDiffSnapshot::ForcedWidth { width } => std::num::NonZeroU16::new(width)
            .map(CellDiffOption::ForcedWidth)
            .ok_or_else(|| ArtifactError::new("forced cell width must be nonzero")),
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// One complete styled terminal cell.
pub struct StyledCellSnapshot {
    /// The absolute terminal column.
    pub x: u16,
    /// The absolute terminal row.
    pub y: u16,
    /// The complete grapheme stored in the cell.
    pub symbol: String,
    /// The foreground color.
    pub foreground: ColorSnapshot,
    /// The background color.
    pub background: ColorSnapshot,
    /// The underline color.
    pub underline: ColorSnapshot,
    /// The modifiers in canonical order.
    pub modifiers: Vec<ModifierSnapshot>,
    /// The cell diff behavior.
    pub diff: CellDiffSnapshot,
    /// The legacy Ratatui skip flag.
    pub skip: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// A complete styled `TestBackend` frame.
pub struct StyledFrameSnapshot {
    /// The complete buffer area.
    pub area: RectSnapshot,
    /// The cursor position.
    pub cursor_position: PositionSnapshot,
    /// Whether the cursor is visible.
    pub cursor_visible: bool,
    /// Every buffer cell in row-major order.
    pub cells: Vec<StyledCellSnapshot>,
}

impl StyledFrameSnapshot {
    #[allow(deprecated)]
    /// Capture the complete buffer, style, diff, and cursor state.
    pub fn from_backend(backend: &TestBackend) -> Self {
        let buffer = backend.buffer();
        Self::from_buffer(buffer, backend.cursor_position(), backend.cursor_visible())
    }

    #[allow(deprecated)]
    /// Capture one logical render buffer with its final cursor state.
    pub fn from_buffer(buffer: &Buffer, cursor_position: Position, cursor_visible: bool) -> Self {
        let area = buffer.area;
        let width = usize::from(area.width);
        let cells = buffer
            .content()
            .iter()
            .enumerate()
            .map(|(index, cell)| StyledCellSnapshot {
                x: area.x + u16::try_from(index % width.max(1)).unwrap_or(0),
                y: area.y + u16::try_from(index / width.max(1)).unwrap_or(0),
                symbol: cell.symbol().to_owned(),
                foreground: cell.fg.into(),
                background: cell.bg.into(),
                underline: cell.underline_color.into(),
                modifiers: modifier_snapshot(cell.modifier),
                diff: cell.diff_option.into(),
                skip: cell.skip,
            })
            .collect();
        Self {
            area: area.into(),
            cursor_position: cursor_position.into(),
            cursor_visible,
            cells,
        }
    }

    #[allow(deprecated)]
    /// Rebuild the complete validated Ratatui buffer without changing cell semantics.
    pub fn to_buffer(&self) -> Result<Buffer, ArtifactError> {
        validate_styled_frame(self)?;
        let mut buffer = Buffer::empty(Rect::new(
            self.area.x,
            self.area.y,
            self.area.width,
            self.area.height,
        ));
        for source in &self.cells {
            let target = buffer
                .cell_mut(Position::new(source.x, source.y))
                .ok_or_else(|| ArtifactError::new("styled frame cell is outside its area"))?;
            target
                .set_symbol(&source.symbol)
                .set_fg(source.foreground.into())
                .set_bg(source.background.into())
                .set_diff_option(cell_diff_option(source.diff)?)
                .set_skip(source.skip);
            target.underline_color = source.underline.into();
            target.modifier = source
                .modifiers
                .iter()
                .copied()
                .map(Modifier::from)
                .fold(Modifier::empty(), |all, modifier| all | modifier);
        }
        Ok(buffer)
    }

    /// Build trimmed readable lines after full frame validation.
    pub fn readable_lines(&self) -> Result<Vec<String>, ArtifactError> {
        validate_styled_frame(self)?;
        let width = usize::from(self.area.width);
        Ok(self
            .cells
            .chunks(width)
            .map(|row| {
                let mut line = String::new();
                let mut continuation_cells = 0;
                for cell in row {
                    if continuation_cells > 0 {
                        continuation_cells -= 1;
                        continue;
                    }
                    if cell_is_skipped(cell) {
                        line.push(' ');
                        continue;
                    }
                    let occupied = cell_width(cell).max(1);
                    if matches!(cell.diff, CellDiffSnapshot::ForcedWidth { .. }) {
                        let natural = usize::from(cell.symbol.as_str().cell_width());
                        if natural <= occupied {
                            line.push_str(&cell.symbol);
                            line.push_str(&" ".repeat(occupied - natural));
                        } else {
                            line.push_str(&" ".repeat(occupied));
                        }
                    } else {
                        line.push_str(&cell.symbol);
                    }
                    continuation_cells = occupied.saturating_sub(1);
                }
                line.truncate(line.trim_end_matches(' ').len());
                line
            })
            .collect())
    }

    /// Return stored cell symbols that contribute no bytes to readable lines.
    ///
    /// The result is in row-major order. It contains explicit and legacy skip
    /// cells, occupied continuation cells, and forced-width cells whose symbol
    /// is wider than its occupied width. The method validates the complete
    /// frame before it returns any cell references.
    pub fn symbols_omitted_from_readable_lines(
        &self,
    ) -> Result<Vec<&StyledCellSnapshot>, ArtifactError> {
        validate_styled_frame(self)?;
        let width = usize::from(self.area.width);
        let mut omitted = Vec::new();
        for row in self.cells.chunks(width) {
            let mut continuation_cells = 0;
            for cell in row {
                if continuation_cells > 0 {
                    omitted.push(cell);
                    continuation_cells -= 1;
                    continue;
                }
                if cell_is_skipped(cell) {
                    omitted.push(cell);
                    continue;
                }
                let occupied = cell_width(cell).max(1);
                if matches!(cell.diff, CellDiffSnapshot::ForcedWidth { .. })
                    && usize::from(cell.symbol.as_str().cell_width()) > occupied
                {
                    omitted.push(cell);
                }
                continuation_cells = occupied.saturating_sub(1);
            }
        }
        Ok(omitted)
    }
}

fn cell_is_skipped(cell: &StyledCellSnapshot) -> bool {
    matches!(cell.diff, CellDiffSnapshot::Skip)
        || cell.skip && matches!(cell.diff, CellDiffSnapshot::None)
}

fn cell_width(cell: &StyledCellSnapshot) -> usize {
    match cell.diff {
        CellDiffSnapshot::ForcedWidth { width } => usize::from(width),
        CellDiffSnapshot::None | CellDiffSnapshot::Skip | CellDiffSnapshot::AlwaysUpdate => {
            usize::from(cell.symbol.as_str().cell_width())
        }
    }
}

fn modifier_rank(modifier: ModifierSnapshot) -> u8 {
    match modifier {
        ModifierSnapshot::Bold => 0,
        ModifierSnapshot::Dim => 1,
        ModifierSnapshot::Italic => 2,
        ModifierSnapshot::Underlined => 3,
        ModifierSnapshot::SlowBlink => 4,
        ModifierSnapshot::RapidBlink => 5,
        ModifierSnapshot::Reversed => 6,
        ModifierSnapshot::Hidden => 7,
        ModifierSnapshot::CrossedOut => 8,
    }
}

/// Validate all structural, coordinate, cursor, style, and width invariants.
pub fn validate_styled_frame(frame: &StyledFrameSnapshot) -> Result<(), ArtifactError> {
    if frame.area.width == 0 || frame.area.height == 0 {
        return Err(ArtifactError::new("styled frame area must be nonzero"));
    }
    let width = usize::from(frame.area.width);
    let height = usize::from(frame.area.height);
    let expected_cells = width
        .checked_mul(height)
        .ok_or_else(|| ArtifactError::new("styled frame cell count overflows usize"))?;
    if frame.cells.len() != expected_cells {
        return Err(ArtifactError::new(
            "styled frame cell count does not match its area",
        ));
    }
    let right = u32::from(frame.area.x) + u32::from(frame.area.width);
    let bottom = u32::from(frame.area.y) + u32::from(frame.area.height);
    let cursor_x = u32::from(frame.cursor_position.x);
    let cursor_y = u32::from(frame.cursor_position.y);
    if frame.cursor_visible
        && (cursor_x < u32::from(frame.area.x)
            || cursor_x >= right
            || cursor_y < u32::from(frame.area.y)
            || cursor_y >= bottom)
    {
        return Err(ArtifactError::new(
            "styled frame cursor is outside its area",
        ));
    }

    for (index, cell) in frame.cells.iter().enumerate() {
        let expected_x = u32::from(frame.area.x)
            + u32::try_from(index % width)
                .map_err(|_| ArtifactError::new("styled frame x coordinate overflows u32"))?;
        let expected_y = u32::from(frame.area.y)
            + u32::try_from(index / width)
                .map_err(|_| ArtifactError::new("styled frame y coordinate overflows u32"))?;
        if u32::from(cell.x) != expected_x || u32::from(cell.y) != expected_y {
            return Err(ArtifactError::new(
                "styled frame cells are not in row-major order",
            ));
        }
        if matches!(cell.diff, CellDiffSnapshot::ForcedWidth { width: 0 }) {
            return Err(ArtifactError::new("forced cell width must be nonzero"));
        }
        if !cell
            .modifiers
            .windows(2)
            .all(|pair| modifier_rank(pair[0]) < modifier_rank(pair[1]))
        {
            return Err(ArtifactError::new(
                "styled frame modifiers are not canonical",
            ));
        }
    }

    for row in frame.cells.chunks(width) {
        let mut continuation_cells = 0;
        for cell in row {
            if continuation_cells > 0 {
                continuation_cells -= 1;
                continue;
            }
            if cell_is_skipped(cell) {
                continue;
            }
            let occupied = cell_width(cell);
            if occupied > row.len().saturating_sub(usize::from(cell.x - frame.area.x)) {
                return Err(ArtifactError::new(format!(
                    "cell width extends past the frame row: row={} x={} symbol={:?} width={occupied}",
                    cell.y, cell.x, cell.symbol,
                )));
            }
            continuation_cells = occupied.saturating_sub(1);
        }
        debug_assert_eq!(continuation_cells, 0);
    }
    Ok(())
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// The state transition boundary represented by one checkpoint.
pub enum TimelineBoundary {
    /// The initial rendered state.
    Initial,
    /// A terminal-session event without a reducer action.
    Session,
    /// A user action after reducer application and before host service.
    UserAction,
    /// A host response after reducer application.
    HostAction,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// The primary operation or final liveness phase.
pub enum TimelinePhase {
    /// Checkpoints caused by the explicit operation vector.
    Operations,
    /// Synthetic checkpoints used to settle the final UI state.
    FinalLiveness,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// The stable identity of one terminal event and its complete effect chain.
pub struct EventChainIdentity {
    /// The phase that owns the event.
    pub phase: TimelinePhase,
    /// The zero-based dispatched-event ordinal within the phase.
    pub sequence: u32,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "result")]
/// The terminal result of the final liveness phase.
pub enum LivenessResult {
    /// The UI settled successfully.
    Passed,
    /// The UI did not settle.
    Failed {
        /// The deterministic failure message.
        message: String,
    },
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", tag = "cause")]
/// The complete cause and outputs for one transition.
pub enum TransitionCause {
    /// The initial state has no incoming event.
    Initial,
    /// A session-only event was consumed or ignored.
    Session {
        /// The source operation.
        requested: Value,
        /// The complete resolved operation plan.
        resolved: Value,
        /// The exact terminal event dispatched for this chain.
        event: Value,
        /// The terminal-session handling result.
        handling: Value,
    },
    /// A terminal event produced a reducer action.
    Reducer {
        /// The source operation.
        requested: Value,
        /// The complete resolved operation plan.
        resolved: Value,
        /// The exact terminal event dispatched for this chain.
        event: Value,
        /// The reducer action.
        action: Value,
        /// The effect emitted by the reducer.
        emitted: Value,
    },
    /// A host request produced a response action.
    Host {
        /// The explicit operation index, or none during final liveness.
        operation_index: Option<u32>,
        /// The host round within one effect chain.
        round: u16,
        /// The effect sent to the host.
        request: Value,
        /// The action returned by the host.
        response: Value,
        /// The next effect emitted by the reducer.
        emitted: Value,
    },
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// Whether a checkpoint has a terminal presentation to review.
pub enum Presentation {
    /// The row contains a rendered terminal frame for its transition.
    Presented,
    /// The transition continued or quit before it rendered a frame.
    NotPresented,
}

/// Classify whether one transition produced a terminal presentation.
#[must_use]
pub fn expected_presentation(row: &TimelineRow) -> Presentation {
    match &row.cause {
        TransitionCause::Session {
            resolved, handling, ..
        } if row.phase == TimelinePhase::FinalLiveness
            && handling == "already_terminal"
            && resolved.get("terminal").and_then(Value::as_str) == Some("quit") =>
        {
            Presentation::NotPresented
        }
        TransitionCause::Initial | TransitionCause::Session { .. } => Presentation::Presented,
        TransitionCause::Reducer { emitted, .. } | TransitionCause::Host { emitted, .. }
            if emitted == "none" =>
        {
            Presentation::Presented
        }
        TransitionCause::Reducer { .. } | TransitionCause::Host { .. } => {
            Presentation::NotPresented
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
/// One complete primary walker checkpoint.
pub struct TimelineRow {
    /// The row schema version.
    pub schema: u16,
    /// The stable review profile identifier.
    pub profile: SafeProfileId,
    /// The zero-based checkpoint sequence.
    pub sequence: u32,
    /// The trace phase.
    pub phase: TimelinePhase,
    /// The terminal event chain, or none for the initial checkpoint.
    pub event_chain: Option<EventChainIdentity>,
    /// The explicit operation index, when one caused the checkpoint.
    pub operation_index: Option<u32>,
    /// The transition boundary.
    pub boundary: TimelineBoundary,
    /// The complete transition cause.
    pub cause: TransitionCause,
    /// Whether this transition produced a terminal presentation.
    pub presentation: Presentation,
    /// The complete reducer-state object reference.
    pub reducer: ObjectRef,
    /// The complete host-state object reference.
    pub host: ObjectRef,
    /// The complete session-state object reference.
    pub session: ObjectRef,
    /// The complete styled-frame object reference.
    pub styled_frame: ObjectRef,
    /// The complete geometry object reference.
    pub geometry: ObjectRef,
    /// The live presentation locale at this checkpoint.
    pub locale: String,
    /// The viewport at this checkpoint.
    pub viewport: RectSnapshot,
    /// The terminal liveness result, present only on the last row.
    pub liveness: Option<LivenessResult>,
    /// The digest of the previous timeline row.
    pub previous_row_sha256: Option<String>,
}

/// Compute the canonical SHA-256 digest for one timeline row.
pub fn timeline_row_digest(row: &TimelineRow) -> Result<String, ArtifactError> {
    let value = serde_json::to_value(row)?;
    Ok(sha256_hex(&canonical_json_bytes(&value)?))
}

fn validate_chain_rows(rows: &[TimelineRow]) -> Result<(), ArtifactError> {
    debug_assert!(!rows.is_empty());
    let first = &rows[0];
    match first.boundary {
        TimelineBoundary::Session if rows.len() == 1 => Ok(()),
        TimelineBoundary::Session => Err(ArtifactError::new(
            "a session-only event chain has more than one checkpoint",
        )),
        TimelineBoundary::UserAction => {
            for (expected_round, row) in rows.iter().skip(1).enumerate() {
                let TransitionCause::Host { round, .. } = row.cause else {
                    return Err(ArtifactError::new(
                        "a user event chain contains a non-host continuation",
                    ));
                };
                if row.boundary != TimelineBoundary::HostAction
                    || usize::from(round) != expected_round
                {
                    return Err(ArtifactError::new(
                        "host rounds do not start at zero and increase by one",
                    ));
                }
            }
            Ok(())
        }
        TimelineBoundary::Initial | TimelineBoundary::HostAction => Err(ArtifactError::new(
            "an event chain must start with a session or user checkpoint",
        )),
    }
}

fn validate_event_chains(rows: &[TimelineRow]) -> Result<(), ArtifactError> {
    let mut index = 0;
    let mut next_operation_chain = 0;
    let mut next_liveness_chain = 0;
    while index < rows.len() {
        let identity = rows[index]
            .event_chain
            .ok_or_else(|| ArtifactError::new("noninitial checkpoint has no event chain"))?;
        match identity.phase {
            TimelinePhase::Operations if identity.sequence == next_operation_chain => {
                next_operation_chain += 1;
            }
            TimelinePhase::Operations => {
                return Err(ArtifactError::new(
                    "operation event chains decrease or skip an identity",
                ));
            }
            TimelinePhase::FinalLiveness if identity.sequence == next_liveness_chain => {
                next_liveness_chain += 1;
            }
            TimelinePhase::FinalLiveness => {
                return Err(ArtifactError::new(
                    "final liveness event chains decrease or skip an identity",
                ));
            }
        }
        let end = rows[index..]
            .iter()
            .position(|row| row.event_chain != Some(identity))
            .map_or(rows.len(), |offset| index + offset);
        let operation_index = rows[index].operation_index;
        if rows[index..end]
            .iter()
            .any(|row| row.phase != identity.phase || row.operation_index != operation_index)
        {
            return Err(ArtifactError::new(
                "one event chain changes phase or operation index",
            ));
        }
        validate_chain_rows(&rows[index..end])?;
        index = end;
    }
    Ok(())
}

/// Validate the complete timeline state machine and hash chain.
pub fn validate_timeline(rows: &[TimelineRow], operation_count: u32) -> Result<(), ArtifactError> {
    let Some(first) = rows.first() else {
        return Err(ArtifactError::new(
            "timeline must contain its initial checkpoint",
        ));
    };
    if first.sequence != 0
        || first.phase != TimelinePhase::Operations
        || first.event_chain.is_some()
        || first.operation_index.is_some()
        || first.boundary != TimelineBoundary::Initial
        || !matches!(first.cause, TransitionCause::Initial)
        || first.previous_row_sha256.is_some()
    {
        return Err(ArtifactError::new(
            "timeline has an invalid initial checkpoint",
        ));
    }
    let profile = &first.profile;
    let mut final_liveness_started = false;
    let mut current_operation = None;
    for (index, row) in rows.iter().enumerate() {
        if row.schema != 3 {
            return Err(ArtifactError::new("timeline row has an unsupported schema"));
        }
        if row.presentation != expected_presentation(row) {
            return Err(ArtifactError::new(
                "timeline row has an incorrect presentation label",
            ));
        }
        if row.profile != *profile || usize::try_from(row.sequence).ok() != Some(index) {
            return Err(ArtifactError::new(
                "timeline sequence or profile is not contiguous",
            ));
        }
        if row.phase == TimelinePhase::FinalLiveness {
            final_liveness_started = true;
        } else if final_liveness_started {
            return Err(ArtifactError::new(
                "operations cannot resume after final liveness starts",
            ));
        }
        let operation_index_is_valid = match row.phase {
            TimelinePhase::Operations if index == 0 => row.operation_index.is_none(),
            TimelinePhase::Operations => {
                let Some(operation_index) = row.operation_index else {
                    return Err(ArtifactError::new(
                        "operation checkpoint has no operation index",
                    ));
                };
                if operation_index >= operation_count
                    || !row
                        .event_chain
                        .is_some_and(|identity| identity.phase == TimelinePhase::Operations)
                {
                    false
                } else {
                    match current_operation {
                        None if operation_index == 0 => current_operation = Some(0),
                        Some(current) if operation_index == current => {}
                        Some(current) if operation_index == current + 1 => {
                            current_operation = Some(operation_index);
                        }
                        None | Some(_) => {
                            return Err(ArtifactError::new(
                                "operation indexes decrease or skip an operation",
                            ));
                        }
                    }
                    true
                }
            }
            TimelinePhase::FinalLiveness => {
                row.operation_index.is_none()
                    && row
                        .event_chain
                        .is_some_and(|identity| identity.phase == TimelinePhase::FinalLiveness)
            }
        };
        let cause_matches_boundary = matches!(
            (&row.boundary, &row.cause),
            (TimelineBoundary::Initial, TransitionCause::Initial)
                | (TimelineBoundary::Session, TransitionCause::Session { .. })
                | (
                    TimelineBoundary::UserAction,
                    TransitionCause::Reducer { .. }
                )
                | (TimelineBoundary::HostAction, TransitionCause::Host { .. })
        );
        if !cause_matches_boundary
            || !operation_index_is_valid
            || (index > 0
                && matches!(
                    (&row.boundary, &row.cause),
                    (TimelineBoundary::Initial, _) | (_, TransitionCause::Initial)
                ))
            || matches!(
                &row.cause,
                TransitionCause::Host {
                    operation_index,
                    ..
                } if *operation_index != row.operation_index
            )
        {
            return Err(ArtifactError::new(
                "timeline boundary does not match its transition cause",
            ));
        }
        let last = index + 1 == rows.len();
        if (!last && row.liveness.is_some())
            || (last && !matches!(row.liveness, Some(LivenessResult::Passed)))
        {
            return Err(ArtifactError::new(
                "terminal liveness result is absent or did not pass",
            ));
        }
        for (reference, kind) in [
            (&row.reducer, ObjectKind::Reducer),
            (&row.host, ObjectKind::Host),
            (&row.session, ObjectKind::Session),
            (&row.styled_frame, ObjectKind::StyledFrame),
            (&row.geometry, ObjectKind::Geometry),
        ] {
            if reference.kind != kind {
                return Err(ArtifactError::new(
                    "timeline object kind is in the wrong column",
                ));
            }
            validate_digest(&reference.sha256)?;
        }
        if index > 0 {
            let expected = timeline_row_digest(&rows[index - 1])?;
            if row.previous_row_sha256.as_deref() != Some(expected.as_str()) {
                return Err(ArtifactError::new(
                    "timeline predecessor digest does not match",
                ));
            }
        }
    }
    validate_event_chains(&rows[1..])?;
    if !final_liveness_started
        || operation_count > 0 && current_operation != Some(operation_count - 1)
    {
        return Err(ArtifactError::new(
            "timeline does not cover every operation and final liveness",
        ));
    }
    Ok(())
}

fn response_has_locale(value: &Value, locale: &str) -> bool {
    match value {
        Value::Array(values) => values
            .iter()
            .any(|value| response_has_locale(value, locale)),
        Value::Object(values) => {
            values.get("locale").and_then(Value::as_str) == Some(locale)
                || values
                    .values()
                    .any(|value| response_has_locale(value, locale))
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => false,
    }
}

fn is_exact_synthetic_event(event: &Value, marker: &str) -> bool {
    event.as_object().is_some_and(|object| {
        object.len() == 1 && object.get("synthetic").and_then(Value::as_str) == Some(marker)
    })
}

fn validate_event_list(
    input: &Map<String, Value>,
    event: &Value,
    ordinal: usize,
    label: &str,
) -> Result<usize, ArtifactError> {
    let events = input.get("events").and_then(Value::as_array);
    let Some(events) = events else {
        return Err(ArtifactError::new(format!(
            "resolved {label} plan has no event list"
        )));
    };
    let Some(expected) = events.get(ordinal) else {
        return Err(ArtifactError::new(format!(
            "event chain exceeds its resolved {label} plan"
        )));
    };
    if event != expected {
        return Err(ArtifactError::new(format!(
            "dispatched event does not match its resolved {label} plan"
        )));
    }
    Ok(events.len())
}

fn validate_operation_plan_event(
    resolved: &Value,
    event: &Value,
    ordinal: usize,
) -> Result<usize, ArtifactError> {
    let input = resolved
        .get("input")
        .and_then(Value::as_object)
        .ok_or_else(|| ArtifactError::new("resolved operation has no input plan"))?;
    let kind = input
        .get("kind")
        .and_then(Value::as_str)
        .ok_or_else(|| ArtifactError::new("resolved operation input has no kind"))?;
    match kind {
        "primary_click" | "local_primary_click" => {
            validate_event_list(input, event, ordinal, "click")
        }
        "event_chain" => validate_event_list(input, event, ordinal, "event chain"),
        "event" | "local_event" | "resize" => {
            if ordinal != 0 {
                return Err(ArtifactError::new(
                    "a single-event plan has more than one event chain",
                ));
            }
            let expected = input
                .get("event")
                .ok_or_else(|| ArtifactError::new("resolved single-event plan has no event"))?;
            if event != expected {
                return Err(ArtifactError::new(
                    "dispatched event does not match its resolved single-event plan",
                ));
            }
            Ok(1)
        }
        "not_applicable" => {
            if ordinal != 0 || !is_exact_synthetic_event(event, "not_applicable") {
                return Err(ArtifactError::new(
                    "a no-op must have one exact synthetic event",
                ));
            }
            Ok(1)
        }
        _ => Err(ArtifactError::new(
            "resolved operation input has an unsupported event plan",
        )),
    }
}

fn validate_liveness_event(cause: &TransitionCause) -> Result<(), ArtifactError> {
    let (resolved, event, already_terminal) = match cause {
        TransitionCause::Session {
            resolved,
            event,
            handling,
            ..
        } => (resolved, event, handling == "already_terminal"),
        TransitionCause::Reducer {
            resolved, event, ..
        } => (resolved, event, false),
        TransitionCause::Initial | TransitionCause::Host { .. } => {
            return Err(ArtifactError::new(
                "a liveness event chain has no dispatched event",
            ));
        }
    };
    if already_terminal {
        let terminal = resolved.get("terminal").and_then(Value::as_str);
        if !matches!(terminal, Some("library" | "quit"))
            || !is_exact_synthetic_event(event, "already_terminal")
        {
            return Err(ArtifactError::new(
                "an already-terminal liveness chain has an invalid result or synthetic event",
            ));
        }
    } else if resolved.get("event") != Some(event) {
        return Err(ArtifactError::new(
            "liveness event does not match its resolved event",
        ));
    }
    Ok(())
}

fn validate_dispatch_events(rows: &[TimelineRow]) -> Result<(), ArtifactError> {
    let mut index = 1;
    let mut operation_index = None;
    let mut operation_plan: Option<&Value> = None;
    let mut dispatched = 0;
    let mut planned = 0;
    while index < rows.len() {
        let identity = rows[index]
            .event_chain
            .ok_or_else(|| ArtifactError::new("noninitial checkpoint has no event chain"))?;
        let end = rows[index..]
            .iter()
            .position(|row| row.event_chain != Some(identity))
            .map_or(rows.len(), |offset| index + offset);
        let cause = &rows[index].cause;
        match identity.phase {
            TimelinePhase::Operations => {
                let current = rows[index]
                    .operation_index
                    .ok_or_else(|| ArtifactError::new("event chain has no operation index"))?;
                if operation_index != Some(current) {
                    if operation_index.is_some() && dispatched != planned {
                        return Err(ArtifactError::new(
                            "operation event chains do not cover its resolved event plan",
                        ));
                    }
                    operation_index = Some(current);
                    operation_plan = None;
                    dispatched = 0;
                }
                let (resolved, event) = match cause {
                    TransitionCause::Session {
                        resolved, event, ..
                    }
                    | TransitionCause::Reducer {
                        resolved, event, ..
                    } => (resolved, event),
                    TransitionCause::Initial | TransitionCause::Host { .. } => {
                        return Err(ArtifactError::new(
                            "an operation event chain has no dispatched event",
                        ));
                    }
                };
                if operation_plan.is_some_and(|plan| plan != resolved) {
                    return Err(ArtifactError::new(
                        "one operation has inconsistent resolved event plans",
                    ));
                }
                operation_plan = Some(resolved);
                let expected_count = validate_operation_plan_event(resolved, event, dispatched)?;
                planned = expected_count;
                dispatched += 1;
            }
            TimelinePhase::FinalLiveness => {
                if operation_index.take().is_some() && dispatched != planned {
                    return Err(ArtifactError::new(
                        "operation event chains do not cover its resolved event plan",
                    ));
                }
                operation_plan = None;
                dispatched = 0;
                planned = 0;
                validate_liveness_event(cause)?;
            }
        }
        index = end;
    }
    if operation_index.is_some() && dispatched != planned {
        return Err(ArtifactError::new(
            "operation event chains do not cover its resolved event plan",
        ));
    }
    Ok(())
}

/// Validate operation payloads, effect causality, and locale transitions.
pub fn validate_timeline_semantics(
    rows: &[TimelineRow],
    operations: &[Value],
    final_liveness_requested: &Value,
    initial_locale: &str,
) -> Result<(), ArtifactError> {
    let operation_count = u32::try_from(operations.len())
        .map_err(|_| ArtifactError::new("operation count exceeds u32"))?;
    validate_timeline(rows, operation_count)?;
    let first = rows
        .first()
        .ok_or_else(|| ArtifactError::new("timeline has no initial locale"))?;
    if first.locale != initial_locale {
        return Err(ArtifactError::new(
            "timeline initial locale does not match its profile",
        ));
    }
    validate_dispatch_events(rows)?;

    let mut previous_locale = first.locale.as_str();
    let mut expected_host_request: Option<&Value> = None;
    for row in rows.iter().skip(1) {
        if row.locale != previous_locale {
            let locale_is_emitted = matches!(
                &row.cause,
                TransitionCause::Host { response, .. }
                    if response_has_locale(response, &row.locale)
            );
            if row.boundary != TimelineBoundary::HostAction || !locale_is_emitted {
                return Err(ArtifactError::new(
                    "timeline locale changed without a matching host response",
                ));
            }
        }
        previous_locale = &row.locale;

        let expected_requested = match row.phase {
            TimelinePhase::Operations => row
                .operation_index
                .and_then(|index| usize::try_from(index).ok())
                .and_then(|index| operations.get(index))
                .ok_or_else(|| ArtifactError::new("operation payload index is invalid"))?,
            TimelinePhase::FinalLiveness => final_liveness_requested,
        };
        if let TransitionCause::Session { requested, .. } = &row.cause
            && requested != expected_requested
        {
            return Err(ArtifactError::new(
                "session requested payload does not match its source operation",
            ));
        }
        if let TransitionCause::Reducer {
            requested, emitted, ..
        } = &row.cause
        {
            if requested != expected_requested {
                return Err(ArtifactError::new(
                    "reducer requested payload does not match its source operation",
                ));
            }
            expected_host_request = Some(emitted);
        }
        if let TransitionCause::Host {
            request, emitted, ..
        } = &row.cause
        {
            if expected_host_request != Some(request) {
                return Err(ArtifactError::new(
                    "host request does not match its predecessor effect",
                ));
            }
            expected_host_request = Some(emitted);
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
/// One contiguous review chunk over timeline rows.
pub struct ChunkDescriptor {
    /// The globally unique chunk identifier.
    pub id: String,
    /// The owning review profile.
    pub profile: SafeProfileId,
    /// The first included row sequence.
    pub start_sequence: u32,
    /// The exclusive row sequence bound.
    pub end_sequence: u32,
    /// The number of included rows.
    pub row_count: u32,
    /// The digest of the canonical row array.
    pub sha256: String,
    /// The digest of the first row.
    pub first_row_sha256: String,
    /// The digest of the last row.
    pub last_row_sha256: String,
    /// The first event chain represented in the chunk.
    pub first_chain: Option<EventChainIdentity>,
    /// The last event chain represented in the chunk.
    pub last_chain: Option<EventChainIdentity>,
    /// Whether the first row continues a chain from the previous chunk.
    pub continues_previous_chain: bool,
    /// Whether the last row continues into the next chunk.
    pub continues_next_chain: bool,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Metadata for one required locale and viewport profile.
pub struct ProfileManifest {
    /// The stable profile identifier.
    pub id: SafeProfileId,
    /// The locale tag at the first checkpoint.
    pub initial_locale: String,
    /// The initial viewport.
    pub initial_viewport: RectSnapshot,
    /// The shared explicit operation-vector digest.
    pub operations_sha256: String,
    /// The digest of the complete asciicast v3 file.
    pub cast_sha256: String,
    /// The byte size of the complete asciicast v3 file.
    pub cast_byte_size: u64,
    /// The total number of timeline rows.
    pub row_count: u32,
    /// The hard maximum number of rows in one chunk.
    pub maximum_chunk_rows: u32,
    /// The complete ordered chunk list.
    pub chunks: Vec<ChunkDescriptor>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// The complete review-corpus metadata.
pub struct CorpusManifest {
    /// The manifest schema version.
    pub schema: u16,
    /// The shared explicit operation-vector digest.
    pub operations_sha256: String,
    /// The exact required review profiles.
    pub profiles: Vec<ProfileManifest>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
/// Local progress for deterministic chunk review.
pub struct ReviewProgress {
    /// The digest of the reviewed manifest.
    pub manifest_sha256: String,
    /// Whether every declared chunk is reviewed.
    pub complete: bool,
    /// The digest of the review report, when it exists.
    pub review_sha256: Option<String>,
    /// Reviewed chunk identifiers mapped to pinned chunk digests.
    pub reviewed_chunks: BTreeMap<String, String>,
    /// The final verdict for each reviewed profile.
    pub profile_verdicts: BTreeMap<SafeProfileId, ReviewVerdict>,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
/// The final review verdict for one profile.
pub enum ReviewVerdict {
    /// The reviewer found no UI issue in the profile.
    NoFindings,
    /// The review report contains one or more findings for the profile.
    FindingsInReport,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// One required locale and viewport in the review corpus.
pub struct RequiredReviewProfile {
    /// The stable profile identifier.
    pub id: &'static str,
    /// The initial locale tag.
    pub locale: &'static str,
    /// The initial terminal viewport.
    pub viewport: RectSnapshot,
}

const REQUIRED_PROFILES: [RequiredReviewProfile; 4] = [
    RequiredReviewProfile {
        id: "en-80x24",
        locale: "en",
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 80,
            height: 24,
        },
    },
    RequiredReviewProfile {
        id: "zh-cn-120x30",
        locale: "zh-CN",
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 120,
            height: 30,
        },
    },
    RequiredReviewProfile {
        id: "zh-tw-40x40",
        locale: "zh-TW",
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 40,
            height: 40,
        },
    },
    RequiredReviewProfile {
        id: "pseudo-120x12",
        locale: "x-pseudo",
        viewport: RectSnapshot {
            x: 0,
            y: 0,
            width: 120,
            height: 12,
        },
    },
];

/// Return the required review profiles in their canonical manifest order.
#[must_use]
pub const fn required_review_profiles() -> &'static [RequiredReviewProfile; 4] {
    &REQUIRED_PROFILES
}

fn chunk_rows_digest(rows: &[TimelineRow]) -> Result<String, ArtifactError> {
    let value = serde_json::to_value(rows)?;
    let bytes = canonical_json_bytes(&value)?;
    Ok(sha256_hex(&bytes))
}

fn chunk_descriptor(
    profile: &SafeProfileId,
    ordinal: usize,
    start: usize,
    end: usize,
    rows: &[TimelineRow],
) -> Result<ChunkDescriptor, ArtifactError> {
    let chunk_rows = &rows[start..end];
    let first = chunk_rows
        .first()
        .ok_or_else(|| ArtifactError::new("chunk must contain at least one row"))?;
    let last = chunk_rows
        .last()
        .ok_or_else(|| ArtifactError::new("chunk must contain at least one row"))?;
    Ok(ChunkDescriptor {
        id: format!("{profile}-{ordinal:04}"),
        profile: profile.clone(),
        start_sequence: u32::try_from(start)
            .map_err(|_| ArtifactError::new("chunk start sequence overflows u32"))?,
        end_sequence: u32::try_from(end)
            .map_err(|_| ArtifactError::new("chunk end sequence overflows u32"))?,
        row_count: u32::try_from(chunk_rows.len())
            .map_err(|_| ArtifactError::new("chunk row count overflows u32"))?,
        sha256: chunk_rows_digest(chunk_rows)?,
        first_row_sha256: timeline_row_digest(first)?,
        last_row_sha256: timeline_row_digest(last)?,
        first_chain: first.event_chain,
        last_chain: last.event_chain,
        continues_previous_chain: start > 0
            && first.event_chain.is_some()
            && rows[start - 1].event_chain == first.event_chain,
        continues_next_chain: end < rows.len()
            && last.event_chain.is_some()
            && rows[end].event_chain == last.event_chain,
    })
}

/// Build deterministic row-count chunks and record chain continuation at each split.
pub fn build_chunks(
    profile: &SafeProfileId,
    rows: &[TimelineRow],
    maximum_rows: usize,
) -> Result<Vec<ChunkDescriptor>, ArtifactError> {
    if rows.is_empty() || maximum_rows == 0 {
        return Err(ArtifactError::new(
            "chunk builder needs rows and a nonzero row limit",
        ));
    }
    if rows.iter().any(|row| &row.profile != profile) {
        return Err(ArtifactError::new(
            "chunk rows do not belong to the requested profile",
        ));
    }
    for index in 1..rows.len() {
        let expected = timeline_row_digest(&rows[index - 1])?;
        if rows[index].previous_row_sha256.as_deref() != Some(expected.as_str()) {
            return Err(ArtifactError::new(
                "chunk rows do not have a continuous predecessor chain",
            ));
        }
    }
    let mut chunks = Vec::new();
    let mut start = 0;
    while start < rows.len() {
        let end = start.saturating_add(maximum_rows).min(rows.len());
        chunks.push(chunk_descriptor(profile, chunks.len(), start, end, rows)?);
        start = end;
    }
    Ok(chunks)
}

/// Validate chunk coverage, row digests, endpoints, and predecessor links.
pub fn validate_chunks(
    profile: &SafeProfileId,
    rows: &[TimelineRow],
    chunks: &[ChunkDescriptor],
    maximum_rows: usize,
) -> Result<(), ArtifactError> {
    if rows.is_empty() || chunks.is_empty() || maximum_rows == 0 {
        return Err(ArtifactError::new("rows and chunks must be nonempty"));
    }
    if rows.iter().any(|row| &row.profile != profile) {
        return Err(ArtifactError::new(
            "chunk rows do not belong to the requested profile",
        ));
    }
    for index in 1..rows.len() {
        let expected = timeline_row_digest(&rows[index - 1])?;
        if rows[index].previous_row_sha256.as_deref() != Some(expected.as_str()) {
            return Err(ArtifactError::new(
                "chunk rows do not have a continuous predecessor chain",
            ));
        }
    }
    let mut next = 0;
    let mut ids = std::collections::BTreeSet::new();
    for (ordinal, chunk) in chunks.iter().enumerate() {
        if &chunk.profile != profile || !ids.insert(&chunk.id) {
            return Err(ArtifactError::new("chunk profile or id is invalid"));
        }
        for digest in [
            &chunk.sha256,
            &chunk.first_row_sha256,
            &chunk.last_row_sha256,
        ] {
            validate_digest(digest)?;
        }
        if chunk.start_sequence != next
            || chunk.end_sequence <= chunk.start_sequence
            || chunk.row_count != chunk.end_sequence - chunk.start_sequence
            || usize::try_from(chunk.row_count)
                .ok()
                .is_none_or(|count| count > maximum_rows)
        {
            return Err(ArtifactError::new(
                "chunks have a gap, overlap, or invalid row count",
            ));
        }
        let start = usize::try_from(chunk.start_sequence)
            .map_err(|_| ArtifactError::new("chunk start does not fit usize"))?;
        let end = usize::try_from(chunk.end_sequence)
            .map_err(|_| ArtifactError::new("chunk end does not fit usize"))?;
        if end > rows.len() {
            return Err(ArtifactError::new("chunk ends after the timeline"));
        }
        let expected = chunk_descriptor(profile, ordinal, start, end, rows)?;
        if *chunk != expected {
            return Err(ArtifactError::new(
                "chunk digest or checkpoint endpoints do not match its rows",
            ));
        }
        next = chunk.end_sequence;
    }
    if usize::try_from(next).ok() != Some(rows.len()) {
        return Err(ArtifactError::new(
            "chunks do not cover the profile row count",
        ));
    }
    Ok(())
}

fn validate_chunk_ranges(profile: &ProfileManifest) -> Result<(), ArtifactError> {
    if profile.maximum_chunk_rows == 0 {
        return Err(ArtifactError::new(
            "manifest chunk row limit must be nonzero",
        ));
    }
    let mut next = 0;
    let mut ids = std::collections::BTreeSet::new();
    for (index, chunk) in profile.chunks.iter().enumerate() {
        if chunk.profile != profile.id || !ids.insert(&chunk.id) {
            return Err(ArtifactError::new("chunk profile or id is invalid"));
        }
        for digest in [
            &chunk.sha256,
            &chunk.first_row_sha256,
            &chunk.last_row_sha256,
        ] {
            validate_digest(digest)?;
        }
        if chunk.start_sequence != next
            || chunk.end_sequence <= chunk.start_sequence
            || chunk.row_count != chunk.end_sequence - chunk.start_sequence
            || chunk.row_count > profile.maximum_chunk_rows
        {
            return Err(ArtifactError::new(
                "manifest chunk ranges have a gap or overlap",
            ));
        }
        if (index == 0 && chunk.continues_previous_chain)
            || (index + 1 == profile.chunks.len() && chunk.continues_next_chain)
            || (chunk.continues_previous_chain && chunk.first_chain.is_none())
            || (chunk.continues_next_chain && chunk.last_chain.is_none())
            || (index == 0 && chunk.first_chain.is_some())
            || (index > 0 && chunk.first_chain.is_none())
            || chunk.last_chain.is_none()
        {
            return Err(ArtifactError::new(
                "manifest chunk continuation metadata is invalid",
            ));
        }
        if let Some(next_chunk) = profile.chunks.get(index + 1) {
            let same_chain = chunk.last_chain == next_chunk.first_chain;
            require(
                chunk.continues_next_chain == same_chain
                    && next_chunk.continues_previous_chain == same_chain,
                "manifest chunk continuation mismatch",
            )?;
        }
        next = chunk.end_sequence;
    }
    if next != profile.row_count {
        return Err(ArtifactError::new(
            "manifest chunks do not cover their profile rows",
        ));
    }
    Ok(())
}

/// Validate the exact four-profile corpus manifest.
pub fn validate_manifest(manifest: &CorpusManifest) -> Result<(), ArtifactError> {
    if manifest.schema != 1 {
        return Err(ArtifactError::new("manifest has an unsupported schema"));
    }
    validate_digest(&manifest.operations_sha256)?;
    if manifest.profiles.len() != required_review_profiles().len() {
        return Err(ArtifactError::new(
            "manifest must contain the exact four review profiles",
        ));
    }
    let mut chunk_ids = std::collections::BTreeSet::new();
    for (profile, required) in manifest.profiles.iter().zip(required_review_profiles()) {
        let profile_id = SafeProfileId::try_from(required.id)
            .map_err(|error| ArtifactError::new(error.to_string()))?;
        require(
            profile.id == profile_id,
            "manifest profiles are not in the required order",
        )?;
        bundle::validate_profile_manifest(
            profile,
            bundle::ProfileExpectation {
                id: &profile_id,
                locale: required.locale,
                viewport: required.viewport,
                operations_sha256: &manifest.operations_sha256,
            },
        )?;
        if profile
            .chunks
            .iter()
            .any(|chunk| !chunk_ids.insert(&chunk.id))
        {
            return Err(ArtifactError::new(
                "manifest chunk ids are not globally unique",
            ));
        }
    }
    Ok(())
}

/// Encode one validated schema-1 corpus manifest as canonical JSON.
pub fn manifest_bytes(manifest: &CorpusManifest) -> Result<Vec<u8>, ArtifactError> {
    validate_manifest(manifest)?;
    canonical_json_bytes(&serde_json::to_value(manifest)?)
}

/// Decode exact canonical schema-1 corpus manifest bytes.
pub fn decode_manifest(bytes: &[u8]) -> Result<CorpusManifest, ArtifactError> {
    let value = bundle::decode_canonical_bytes(bytes)?;
    validate_manifest_wire_shape(&value)?;
    let manifest: CorpusManifest = serde_json::from_value(value)?;
    validate_manifest(&manifest)?;
    Ok(manifest)
}

const MANIFEST_WIRE_FIELDS: &[&str] = &["schema", "operations_sha256", "profiles"];
const PROFILE_WIRE_FIELDS: &[&str] = &[
    "id",
    "initial_locale",
    "initial_viewport",
    "operations_sha256",
    "cast_sha256",
    "cast_byte_size",
    "row_count",
    "maximum_chunk_rows",
    "chunks",
];
const VIEWPORT_WIRE_FIELDS: &[&str] = &["x", "y", "width", "height"];
const CHUNK_WIRE_FIELDS: &[&str] = &[
    "id",
    "profile",
    "start_sequence",
    "end_sequence",
    "row_count",
    "sha256",
    "first_row_sha256",
    "last_row_sha256",
    "first_chain",
    "last_chain",
    "continues_previous_chain",
    "continues_next_chain",
];
const EVENT_CHAIN_WIRE_FIELDS: &[&str] = &["phase", "sequence"];

fn validate_manifest_wire_shape(value: &Value) -> Result<(), ArtifactError> {
    let manifest = exact_wire_object(value, MANIFEST_WIRE_FIELDS, "manifest fields are not exact")?;
    let profiles = manifest["profiles"]
        .as_array()
        .ok_or_else(|| ArtifactError::new("manifest profiles is not an array"))?;
    for profile in profiles {
        let profile = exact_wire_object(
            profile,
            PROFILE_WIRE_FIELDS,
            "manifest profile fields are not exact",
        )?;
        exact_wire_object(
            &profile["initial_viewport"],
            VIEWPORT_WIRE_FIELDS,
            "manifest viewport fields are not exact",
        )?;
        let chunks = profile["chunks"]
            .as_array()
            .ok_or_else(|| ArtifactError::new("manifest chunks is not an array"))?;
        for chunk in chunks {
            let chunk = exact_wire_object(
                chunk,
                CHUNK_WIRE_FIELDS,
                "manifest chunk fields are not exact",
            )?;
            for name in ["first_chain", "last_chain"] {
                if !chunk[name].is_null() {
                    exact_wire_object(
                        &chunk[name],
                        EVENT_CHAIN_WIRE_FIELDS,
                        "manifest event chain fields are not exact",
                    )?;
                }
            }
        }
    }
    Ok(())
}

fn exact_wire_object<'a>(
    value: &'a Value,
    fields: &[&str],
    message: &'static str,
) -> Result<&'a serde_json::Map<String, Value>, ArtifactError> {
    let object = value
        .as_object()
        .ok_or_else(|| ArtifactError::new(message))?;
    let exact =
        object.len() == fields.len() && fields.iter().all(|field| object.contains_key(*field));
    require(exact, message)?;
    Ok(object)
}

/// Compute the canonical corpus-manifest digest.
pub fn manifest_digest(manifest: &CorpusManifest) -> Result<String, ArtifactError> {
    let value = serde_json::to_value(manifest)?;
    let bytes = canonical_json_bytes(&value)?;
    Ok(sha256_hex(&bytes))
}

/// Validate local review progress against the pinned manifest and chunks.
pub fn validate_progress(
    manifest: &CorpusManifest,
    progress: &ReviewProgress,
) -> Result<(), ArtifactError> {
    validate_manifest(manifest)?;
    if progress.manifest_sha256 != manifest_digest(manifest)? {
        return Err(ArtifactError::new(
            "progress refers to a different manifest",
        ));
    }
    let mut declared = BTreeMap::new();
    for profile in &manifest.profiles {
        for chunk in &profile.chunks {
            declared.insert(chunk.id.clone(), chunk.sha256.clone());
        }
    }
    for (id, digest) in &progress.reviewed_chunks {
        if declared.get(id) != Some(digest) {
            return Err(ArtifactError::new(
                "progress has an unknown or stale chunk digest",
            ));
        }
    }
    if let Some(review_sha256) = &progress.review_sha256 {
        validate_digest(review_sha256)?;
    }
    let declared_profiles = manifest
        .profiles
        .iter()
        .map(|profile| &profile.id)
        .collect::<std::collections::BTreeSet<_>>();
    if progress
        .profile_verdicts
        .keys()
        .any(|profile| !declared_profiles.contains(profile))
    {
        return Err(ArtifactError::new(
            "progress has a verdict for an unknown profile",
        ));
    }
    let actually_complete = progress.reviewed_chunks.len() == declared.len();
    if progress.complete != actually_complete {
        return Err(ArtifactError::new(
            "progress completion does not match reviewed chunks",
        ));
    }
    if progress.complete
        && (progress.review_sha256.is_none()
            || progress.profile_verdicts.len() != declared_profiles.len())
    {
        return Err(ArtifactError::new(
            "completed progress needs a report digest and every profile verdict",
        ));
    }
    Ok(())
}

/// Encode validated review progress as canonical JSON with one trailing newline.
pub fn review_progress_bytes(
    manifest: &CorpusManifest,
    progress: &ReviewProgress,
) -> Result<Vec<u8>, ArtifactError> {
    validate_progress(manifest, progress)?;
    let mut bytes = canonical_json_bytes(&serde_json::to_value(progress)?)?;
    bytes.push(b'\n');
    Ok(bytes)
}

/// Decode review progress with canonical JSON and exactly one trailing newline.
pub fn decode_review_progress(
    manifest: &CorpusManifest,
    bytes: &[u8],
) -> Result<ReviewProgress, ArtifactError> {
    if !bytes.ends_with(b"\n") || bytes.ends_with(b"\n\n") {
        return Err(ArtifactError::new(
            "review progress must have exactly one trailing newline",
        ));
    }
    let value = bundle::decode_canonical_bytes(&bytes[..bytes.len() - 1])?;
    let object = value
        .as_object()
        .ok_or_else(|| ArtifactError::new("review progress is not an object"))?;
    require(
        object.contains_key("review_sha256"),
        "review progress is missing review_sha256",
    )?;
    let progress: ReviewProgress = serde_json::from_value(value)?;
    validate_progress(manifest, &progress)?;
    Ok(progress)
}

/// Validate a complete local review claim against its report bytes.
///
/// The report bytes must satisfy `bundle::validate_review_report`.
pub fn validate_completed_review_claim(
    manifest: &CorpusManifest,
    progress: &ReviewProgress,
    report: &[u8],
) -> Result<(), ArtifactError> {
    validate_progress(manifest, progress)?;
    require(
        progress.complete,
        "the local UI review still has pending chunks",
    )?;
    require(
        report != bundle::REVIEW_TEMPLATE,
        "the UI review report is still the unedited template",
    )?;
    bundle::validate_review_report(report)?;
    let report_sha256 = sha256_hex(report);
    require(
        progress.review_sha256.as_deref() == Some(report_sha256.as_str()),
        "the UI review report digest does not match the completion claim",
    )
}

#[cfg(test)]
mod contract_tests;

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::num::NonZeroU16;

    use ratatui_core::{
        backend::{Backend, TestBackend},
        buffer::{Buffer, Cell, CellDiffOption},
        layout::{Position, Rect, Size},
        style::{Color, Modifier, Style},
    };
    use serde_json::{Value, json};

    use super::{
        ChunkDescriptor, CorpusManifest, EventChainIdentity, LivenessResult, ModifierSnapshot,
        ObjectKind, Presentation, ProfileManifest, RectSnapshot, ReviewProgress, ReviewVerdict,
        SafeProfileId, StyledFrameSnapshot, TimelineBoundary, TimelinePhase, TimelineRow,
        TransitionCause, build_chunks, build_object, canonical_json_bytes, manifest_digest,
        object_ref, sha256_hex, timeline_row_digest, validate_asciicast_v3, validate_chunks,
        validate_dispatch_events, validate_liveness_event, validate_manifest, validate_object,
        validate_operation_plan_event, validate_progress, validate_styled_frame, validate_timeline,
        validate_timeline_semantics,
    };

    const ZERO_DIGEST: &str = "0000000000000000000000000000000000000000000000000000000000000000";

    fn profile(value: &str) -> SafeProfileId {
        SafeProfileId::try_from(value).unwrap()
    }

    #[test]
    fn canonical_json_sorts_object_keys_and_hashes_the_canonical_bytes() {
        let left = json!({"z": 1, "a": {"y": 2, "b": 3}});
        let right = json!({"a": {"b": 3, "y": 2}, "z": 1});

        assert_eq!(
            canonical_json_bytes(&left).unwrap(),
            br#"{"a":{"b":3,"y":2},"z":1}"#
        );
        assert_eq!(
            object_ref(ObjectKind::Reducer, &left).unwrap(),
            object_ref(ObjectKind::Reducer, &right).unwrap()
        );
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(
            sha256_hex(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq"),
            "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
        );
    }

    #[test]
    fn object_validation_rejects_corruption_and_kind_substitution() {
        let (reference, bytes) = build_object(ObjectKind::Host, &json!({"entries": 3})).unwrap();
        assert_eq!(
            validate_object(&reference, &bytes).unwrap(),
            json!({"entries": 3})
        );

        let mut corrupt = bytes.clone();
        *corrupt.last_mut().unwrap() ^= 1;
        assert!(validate_object(&reference, &corrupt).is_err());

        let mut wrong_kind = reference.clone();
        wrong_kind.kind = ObjectKind::Reducer;
        assert!(validate_object(&wrong_kind, &bytes).is_err());

        let pretty: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let pretty = serde_json::to_vec_pretty(&pretty).unwrap();
        assert!(validate_object(&reference, &pretty).is_err());

        let mut schema: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        schema["schema"] = json!(2);
        let schema = canonical_json_bytes(&schema).unwrap();
        assert!(validate_object(&reference, &schema).is_err());

        let (_, changed) = build_object(ObjectKind::Host, &json!({"entries": 4})).unwrap();
        assert!(validate_object(&reference, &changed).is_err());

        let invalid_digest = super::ObjectRef {
            kind: ObjectKind::Host,
            sha256: "INVALID".to_owned(),
        };
        let error = validate_object(&invalid_digest, &bytes).unwrap_err();
        assert!(error.to_string().contains("invalid lowercase SHA-256"));
    }

    fn styled_backend(width: u16, height: u16) -> TestBackend {
        let mut backend = TestBackend::new(width, height);
        let mut buffer = Buffer::empty(Rect::new(0, 0, width, height));
        let style = Style::default()
            .fg(Color::Rgb(1, 2, 3))
            .bg(Color::Indexed(42))
            .add_modifier(Modifier::BOLD | Modifier::UNDERLINED);
        buffer.set_string(0, 0, "界 👩\u{200d}💻 e\u{301}", style);
        backend
            .draw(buffer.content.iter().enumerate().map(|(index, cell)| {
                let index = u16::try_from(index).unwrap();
                (index % width, index / width, cell)
            }))
            .unwrap();
        backend.show_cursor().unwrap();
        backend
            .set_cursor_position(Position { x: 2, y: 0 })
            .unwrap();
        backend
    }

    #[test]
    #[allow(deprecated)]
    fn styled_frame_round_trip_keeps_cells_unicode_style_skip_and_cursor() {
        let mut backend = styled_backend(20, 2);
        let mut skipped = Cell::default();
        skipped.set_diff_option(CellDiffOption::Skip).set_skip(true);
        backend.draw(std::iter::once((19, 0, &skipped))).unwrap();
        let snapshot = StyledFrameSnapshot::from_backend(&backend);
        let decoded: StyledFrameSnapshot = serde_json::from_slice(
            &canonical_json_bytes(&serde_json::to_value(&snapshot).unwrap()).unwrap(),
        )
        .unwrap();

        assert_eq!(decoded, snapshot);
        assert!(snapshot.cursor_visible);
        assert_eq!(snapshot.cursor_position.x, 2);
        assert_eq!(snapshot.cells.len(), 40);
        assert!(snapshot.readable_lines().unwrap()[0].contains("界 👩\u{200d}💻 e\u{301}"));
        assert!(snapshot.cells.iter().any(|cell| {
            cell.modifiers.contains(&ModifierSnapshot::Bold)
                && cell.foreground
                    == (super::ColorSnapshot::Rgb {
                        red: 1,
                        green: 2,
                        blue: 3,
                    })
        }));
        assert!(snapshot.cells[19].skip);
        assert_eq!(snapshot.cells[19].diff, super::CellDiffSnapshot::Skip);

        let named = [
            Color::Black,
            Color::Red,
            Color::Green,
            Color::Yellow,
            Color::Blue,
            Color::Magenta,
            Color::Cyan,
            Color::Gray,
            Color::DarkGray,
            Color::LightRed,
            Color::LightGreen,
            Color::LightYellow,
            Color::LightBlue,
            Color::LightMagenta,
            Color::LightCyan,
            Color::White,
        ]
        .map(super::ColorSnapshot::from);
        assert_eq!(named.len(), 16);
    }

    #[test]
    fn styled_frame_resize_round_trip_changes_area_and_cell_count() {
        let mut backend = styled_backend(6, 2);
        let before = StyledFrameSnapshot::from_backend(&backend);
        backend.resize(9, 3);
        let after = StyledFrameSnapshot::from_backend(&backend);

        assert_eq!(before.area.width, 6);
        assert_eq!(before.cells.len(), 12);
        assert_eq!(after.area.width, 9);
        assert_eq!(after.area.height, 3);
        assert_eq!(after.cells.len(), 27);
        let value = serde_json::to_value(&after).unwrap();
        assert_eq!(
            serde_json::from_value::<StyledFrameSnapshot>(value).unwrap(),
            after
        );
        validate_styled_frame(&before).unwrap();
        validate_styled_frame(&after).unwrap();
    }

    #[test]
    fn styled_frame_validator_rejects_structural_and_cursor_corruption() {
        let valid = StyledFrameSnapshot::from_backend(&styled_backend(20, 2));
        validate_styled_frame(&valid).unwrap();

        let mut zero = valid.clone();
        zero.area.width = 0;
        zero.cells.clear();
        assert!(validate_styled_frame(&zero).is_err());

        let mut missing = valid.clone();
        missing.cells.pop();
        assert!(validate_styled_frame(&missing).is_err());

        let mut coordinate = valid.clone();
        coordinate.cells[1].x = 7;
        assert!(validate_styled_frame(&coordinate).is_err());

        let mut cursor = valid.clone();
        cursor.cursor_position.x = cursor.area.width;
        assert!(validate_styled_frame(&cursor).is_err());

        let mut modifiers = valid;
        modifiers.cells[0].modifiers = vec![
            ModifierSnapshot::Bold,
            ModifierSnapshot::Dim,
            ModifierSnapshot::Italic,
            ModifierSnapshot::Underlined,
            ModifierSnapshot::SlowBlink,
            ModifierSnapshot::RapidBlink,
            ModifierSnapshot::Reversed,
            ModifierSnapshot::Hidden,
            ModifierSnapshot::CrossedOut,
        ];
        validate_styled_frame(&modifiers).unwrap();
        modifiers.cells[0].modifiers = vec![ModifierSnapshot::Bold, ModifierSnapshot::Bold];
        assert!(validate_styled_frame(&modifiers).is_err());
        modifiers.cells[0].modifiers = vec![ModifierSnapshot::Underlined, ModifierSnapshot::Bold];
        assert!(validate_styled_frame(&modifiers).is_err());

        let mut forced_zero = modifiers.clone();
        forced_zero.cells[0].modifiers.clear();
        forced_zero.cells[0].diff = super::CellDiffSnapshot::ForcedWidth { width: 0 };
        assert!(validate_styled_frame(&forced_zero).is_err());

        let mut too_wide = modifiers;
        too_wide.cells[0].modifiers.clear();
        let last = too_wide.cells.len() - 1;
        too_wide.cells[last].symbol = "界".to_owned();
        assert!(validate_styled_frame(&too_wide).is_err());
    }

    #[test]
    fn readable_frame_keeps_hidden_cursor_diff_options_and_halfwidth_voicing() {
        let mut backend = TestBackend::new(8, 1);
        backend.hide_cursor().unwrap();
        let mut halfwidth = Cell::default();
        halfwidth.set_symbol("ｶ");
        let mut voicing = Cell::default();
        voicing
            .set_symbol("ﾞ")
            .set_diff_option(CellDiffOption::AlwaysUpdate);
        let mut forced = Cell::default();
        forced
            .set_symbol("x")
            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap()));
        backend
            .draw([(0, 0, &halfwidth), (1, 0, &voicing), (3, 0, &forced)].into_iter())
            .unwrap();
        let mut snapshot = StyledFrameSnapshot::from_backend(&backend);
        snapshot.cells[7].symbol = "q".to_owned();
        snapshot.cells[7].skip = true;

        validate_styled_frame(&snapshot).unwrap();
        assert!(!snapshot.cursor_visible);
        let readable = snapshot.readable_lines().unwrap();
        assert!(readable[0].contains("ｶﾞ"));
        assert!(!readable[0].contains('q'));
        snapshot.cursor_position = Position { x: 999, y: 999 }.into();
        validate_styled_frame(&snapshot).unwrap();
        assert_eq!(
            snapshot.cells[1].diff,
            super::CellDiffSnapshot::AlwaysUpdate
        );
        assert_eq!(
            snapshot.cells[3].diff,
            super::CellDiffSnapshot::ForcedWidth { width: 2 }
        );

        let mut stale_continuation = snapshot;
        stale_continuation.cells[3].symbol = "界".to_owned();
        stale_continuation.cells[4].symbol = "ﾞ".to_owned();
        validate_styled_frame(&stale_continuation).unwrap();
        assert!(!stale_continuation.readable_lines().unwrap()[0].contains("界ﾞ"));

        let mut aligned = TestBackend::new(4, 1);
        let left = Cell::new("a");
        let mut skipped = Cell::new("q");
        skipped.set_diff_option(CellDiffOption::Skip);
        let right = Cell::new("b");
        aligned
            .draw([(0, 0, &left), (1, 0, &skipped), (2, 0, &right)].into_iter())
            .unwrap();
        assert_eq!(
            StyledFrameSnapshot::from_backend(&aligned)
                .readable_lines()
                .unwrap()[0],
            "a b"
        );

        let mut wide_skip = TestBackend::new(3, 1);
        let mut skipped_wide = Cell::new("界");
        skipped_wide.set_diff_option(CellDiffOption::Skip);
        let marker = Cell::new("B");
        wide_skip
            .draw([(0, 0, &skipped_wide), (1, 0, &marker)].into_iter())
            .unwrap();
        assert_eq!(
            StyledFrameSnapshot::from_backend(&wide_skip)
                .readable_lines()
                .unwrap()[0],
            " B"
        );

        let mut forced_alignment = TestBackend::new(4, 1);
        let mut forced = Cell::new("x");
        forced.set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap()));
        let stale = Cell::new("q");
        forced_alignment
            .draw([(0, 0, &forced), (1, 0, &stale), (2, 0, &marker)].into_iter())
            .unwrap();
        assert_eq!(
            StyledFrameSnapshot::from_backend(&forced_alignment)
                .readable_lines()
                .unwrap()[0],
            "x B"
        );

        let mut narrow_forced = TestBackend::new(2, 1);
        let mut forced_wide = Cell::new("界");
        forced_wide.set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()));
        narrow_forced
            .draw(std::iter::once((0, 0, &forced_wide)))
            .unwrap();
        assert_eq!(
            StyledFrameSnapshot::from_backend(&narrow_forced)
                .readable_lines()
                .unwrap()[0],
            ""
        );

        let mut voiced = TestBackend::new(3, 1);
        let katakana = Cell::new("ｶﾞ");
        let stale = Cell::new("x");
        voiced
            .draw([(0, 0, &katakana), (1, 0, &stale)].into_iter())
            .unwrap();
        assert_eq!(
            StyledFrameSnapshot::from_backend(&voiced)
                .readable_lines()
                .unwrap()[0],
            "ｶﾞ"
        );
    }

    #[test]
    fn styled_frame_reports_every_stored_symbol_omitted_from_readable_lines() {
        let area = Rect::new(10, 20, 9, 1);
        let mut buffer = Buffer::empty(area);
        buffer.content[0].set_symbol("V");
        buffer.content[1]
            .set_symbol("diff-skip")
            .set_diff_option(CellDiffOption::Skip);
        buffer.content[2].set_symbol("legacy-skip");
        buffer.content[3].set_symbol("界");
        buffer.content[4].set_symbol("wide-continuation");
        buffer.content[5]
            .set_symbol("界")
            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(1).unwrap()));
        buffer.content[6]
            .set_symbol("F")
            .set_diff_option(CellDiffOption::ForcedWidth(NonZeroU16::new(2).unwrap()));
        buffer.content[7].set_symbol("forced-continuation");
        buffer.content[8].set_symbol("Z");
        let mut snapshot = StyledFrameSnapshot::from_buffer(
            &buffer,
            Position {
                x: area.x,
                y: area.y,
            },
            false,
        );
        snapshot.cells[2].skip = true;

        let omitted = snapshot
            .symbols_omitted_from_readable_lines()
            .unwrap()
            .into_iter()
            .map(|cell| (cell.x, cell.y, cell.symbol.as_str()))
            .collect::<Vec<_>>();

        assert_eq!(
            omitted,
            vec![
                (11, 20, "diff-skip"),
                (12, 20, "legacy-skip"),
                (14, 20, "wide-continuation"),
                (15, 20, "界"),
                (17, 20, "forced-continuation"),
            ]
        );
        assert!(!omitted.iter().any(|(_, _, symbol)| *symbol == "V"));
        assert!(!omitted.iter().any(|(_, _, symbol)| *symbol == "F"));
        assert!(!omitted.iter().any(|(_, _, symbol)| *symbol == "Z"));

        snapshot.cells.pop();
        assert!(snapshot.symbols_omitted_from_readable_lines().is_err());
    }

    #[test]
    fn real_walker_frame_hides_stale_ascii_behind_a_cjk_cell() {
        let mut backend = TestBackend::new(80, 6);
        let cjk = Cell::new("命");
        let stale = Cell::new("y");
        backend
            .draw([(76, 5, &cjk), (77, 5, &stale)].into_iter())
            .unwrap();
        let snapshot = StyledFrameSnapshot::from_backend(&backend);

        validate_styled_frame(&snapshot).unwrap();
        assert_eq!(snapshot.cells[5 * 80 + 76].symbol, "命");
        assert_eq!(snapshot.cells[5 * 80 + 77].symbol, "y");
        let readable = snapshot.readable_lines().unwrap();
        assert!(readable[5].ends_with('命'));
        assert!(!readable[5].contains("命y"));
    }

    #[test]
    fn rectangle_dto_round_trip_keeps_nonzero_origin_and_size() {
        let snapshot = RectSnapshot::from(Rect::new(3, 4, 30, 10));
        let value = serde_json::to_value(snapshot).unwrap();
        assert_eq!(
            serde_json::from_value::<RectSnapshot>(value).unwrap(),
            snapshot
        );
        assert_eq!(snapshot.x, 3);
        assert_eq!(snapshot.y, 4);
        assert_eq!(snapshot.width, 30);
        assert_eq!(snapshot.height, 10);
    }

    fn reference(kind: ObjectKind) -> super::ObjectRef {
        super::ObjectRef {
            kind,
            sha256: ZERO_DIGEST.to_owned(),
        }
    }

    fn resolved_single(event: serde_json::Value) -> serde_json::Value {
        json!({
            "input": {"kind": "event", "event": event},
            "semantic_target": null,
        })
    }

    fn resolved_multi(kind: &str, events: &[serde_json::Value]) -> serde_json::Value {
        json!({
            "input": {"kind": kind, "events": events},
            "semantic_target": null,
        })
    }

    fn dispatched_values(cause: &mut TransitionCause) -> (&mut Value, &mut Value) {
        match cause {
            TransitionCause::Session {
                resolved, event, ..
            }
            | TransitionCause::Reducer {
                resolved, event, ..
            } => (resolved, event),
            TransitionCause::Initial | TransitionCause::Host { .. } => {
                panic!("the test checkpoint must contain a dispatched event");
            }
        }
    }

    #[test]
    #[should_panic(expected = "the test checkpoint must contain a dispatched event")]
    fn dispatched_values_rejects_a_checkpoint_without_a_dispatch() {
        let mut cause = TransitionCause::Initial;
        let _ = dispatched_values(&mut cause);
    }

    fn row(sequence: u32, previous: Option<String>) -> TimelineRow {
        TimelineRow {
            schema: 3,
            profile: profile("en-80x24"),
            sequence,
            phase: TimelinePhase::Operations,
            event_chain: sequence.checked_sub(1).map(|sequence| EventChainIdentity {
                phase: TimelinePhase::Operations,
                sequence,
            }),
            operation_index: sequence.checked_sub(1),
            boundary: if sequence == 0 {
                TimelineBoundary::Initial
            } else {
                TimelineBoundary::Session
            },
            cause: if sequence == 0 {
                TransitionCause::Initial
            } else {
                TransitionCause::Session {
                    requested: json!({"focus": true}),
                    resolved: resolved_single(json!("focus_gained")),
                    event: json!("focus_gained"),
                    handling: json!("ignored"),
                }
            },
            presentation: Presentation::Presented,
            reducer: reference(ObjectKind::Reducer),
            host: reference(ObjectKind::Host),
            session: reference(ObjectKind::Session),
            styled_frame: reference(ObjectKind::StyledFrame),
            geometry: reference(ObjectKind::Geometry),
            locale: "en".to_owned(),
            viewport: RectSnapshot {
                x: 0,
                y: 0,
                width: 80,
                height: 24,
            },
            liveness: None,
            previous_row_sha256: previous,
        }
    }

    #[test]
    fn unchanged_checkpoint_objects_can_repeat_without_dropping_a_timeline_row() {
        let rows = complete_timeline();
        assert_eq!(rows[0].reducer, rows[1].reducer);
        assert_eq!(rows[0].styled_frame, rows[1].styled_frame);
        validate_timeline(&rows, 1).unwrap();
    }

    #[test]
    fn timeline_rejects_missing_or_reordered_checkpoint_rows() {
        let rows = complete_timeline();
        let mut missing = rows.clone();
        missing.remove(1);
        assert!(validate_timeline(&missing, 1).is_err());

        let mut predecessor = rows;
        predecessor[1].previous_row_sha256 = Some(ZERO_DIGEST.to_owned());
        assert!(validate_timeline(&predecessor, 1).is_err());
    }

    #[test]
    fn timeline_rejects_a_cause_under_the_wrong_boundary() {
        let mut rows = complete_timeline();
        rows[1].boundary = TimelineBoundary::Session;
        relink(&mut rows);
        assert!(validate_timeline(&rows, 1).is_err());
    }

    #[test]
    fn final_liveness_rows_are_kept_without_impersonating_vector_operations() {
        let rows = complete_timeline();
        validate_timeline(&rows, 1).unwrap();

        let mut missing_operation = rows.clone();
        missing_operation[1].operation_index = None;
        relink(&mut missing_operation);
        assert!(validate_timeline(&missing_operation, 1).is_err());

        let mut out_of_bounds = rows;
        out_of_bounds.last_mut().unwrap().operation_index = Some(1);
        relink(&mut out_of_bounds);
        assert!(validate_timeline(&out_of_bounds, 1).is_err());
    }

    fn relink(rows: &mut [TimelineRow]) {
        let mut previous = None;
        for (sequence, row) in rows.iter_mut().enumerate() {
            row.sequence = u32::try_from(sequence).unwrap();
            row.previous_row_sha256 = previous;
            previous = Some(timeline_row_digest(row).unwrap());
        }
    }

    fn complete_timeline() -> Vec<TimelineRow> {
        let mut rows = vec![row(0, None), row(1, None), row(2, None), row(3, None)];
        rows[1].operation_index = Some(0);
        rows[1].boundary = TimelineBoundary::UserAction;
        rows[1].cause = TransitionCause::Reducer {
            requested: json!({"operation": 0}),
            resolved: resolved_single(json!("enter")),
            event: json!("enter"),
            action: json!({"open": true}),
            emitted: json!({"open": true}),
        };
        rows[1].presentation = Presentation::NotPresented;
        rows[2].operation_index = Some(0);
        rows[2].event_chain = rows[1].event_chain;
        rows[2].boundary = TimelineBoundary::HostAction;
        rows[2].cause = TransitionCause::Host {
            operation_index: Some(0),
            round: 0,
            request: json!({"open": true}),
            response: json!({"present": true}),
            emitted: json!("none"),
        };
        rows[3].phase = TimelinePhase::FinalLiveness;
        rows[3].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::FinalLiveness,
            sequence: 0,
        });
        rows[3].operation_index = None;
        rows[3].cause = TransitionCause::Session {
            requested: json!({"focus": true}),
            resolved: json!({"terminal": "library"}),
            event: json!({"synthetic": "already_terminal"}),
            handling: json!("already_terminal"),
        };
        rows[3].liveness = Some(LivenessResult::Passed);
        relink(&mut rows);
        rows
    }

    fn two_chain_click_timeline() -> Vec<TimelineRow> {
        let click = resolved_multi("primary_click", &[json!("mouse_down"), json!("mouse_up")]);
        let mut rows = vec![row(0, None), row(1, None), row(2, None), row(3, None)];
        rows[1].operation_index = Some(0);
        rows[1].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 0,
        });
        rows[1].cause = TransitionCause::Session {
            requested: json!({"operation": "click"}),
            resolved: click.clone(),
            event: json!("mouse_down"),
            handling: json!("consumed"),
        };
        rows[2].operation_index = Some(0);
        rows[2].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 1,
        });
        rows[2].boundary = TimelineBoundary::UserAction;
        rows[2].cause = TransitionCause::Reducer {
            requested: json!({"operation": "click"}),
            resolved: click,
            event: json!("mouse_up"),
            action: json!("activate"),
            emitted: json!("none"),
        };
        rows[3].phase = TimelinePhase::FinalLiveness;
        rows[3].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::FinalLiveness,
            sequence: 0,
        });
        rows[3].operation_index = None;
        rows[3].cause = TransitionCause::Session {
            requested: json!({"synthetic": "final_liveness"}),
            resolved: json!({"terminal": "library"}),
            event: json!({"synthetic": "already_terminal"}),
            handling: json!("already_terminal"),
        };
        rows[3].liveness = Some(LivenessResult::Passed);
        relink(&mut rows);
        rows
    }

    fn three_chain_event_timeline() -> Vec<TimelineRow> {
        let requested = json!({"operation": "event_chain"});
        let resolved = resolved_multi(
            "event_chain",
            &[json!("first"), json!("second"), json!("third")],
        );
        let mut rows = vec![
            row(0, None),
            row(1, None),
            row(2, None),
            row(3, None),
            row(4, None),
        ];
        for (sequence, event) in ["first", "second", "third"].into_iter().enumerate() {
            let row = &mut rows[sequence + 1];
            row.operation_index = Some(0);
            row.event_chain = Some(EventChainIdentity {
                phase: TimelinePhase::Operations,
                sequence: u32::try_from(sequence).unwrap(),
            });
            row.cause = TransitionCause::Session {
                requested: requested.clone(),
                resolved: resolved.clone(),
                event: json!(event),
                handling: json!("consumed"),
            };
        }
        let liveness = rows.last_mut().unwrap();
        liveness.phase = TimelinePhase::FinalLiveness;
        liveness.event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::FinalLiveness,
            sequence: 0,
        });
        liveness.operation_index = None;
        liveness.cause = TransitionCause::Session {
            requested: json!({"synthetic": "final_liveness"}),
            resolved: json!({"terminal": "library"}),
            event: json!({"synthetic": "already_terminal"}),
            handling: json!("already_terminal"),
        };
        liveness.liveness = Some(LivenessResult::Passed);
        relink(&mut rows);
        rows
    }

    fn set_event_chain_plan(rows: &mut [TimelineRow], events: Value) {
        for row in &mut rows[1..=3] {
            let (resolved, _) = dispatched_values(&mut row.cause);
            resolved["input"]["events"] = events.clone();
        }
        relink(rows);
    }

    fn event_chain_error(rows: &[TimelineRow]) -> String {
        validate_timeline_semantics(
            rows,
            &[json!({"operation": "event_chain"})],
            &json!({"synthetic": "final_liveness"}),
            "en",
        )
        .unwrap_err()
        .to_string()
    }

    #[test]
    fn timeline_accepts_three_ordered_dispatch_chains_for_one_event_chain_operation() {
        validate_timeline_semantics(
            &three_chain_event_timeline(),
            &[json!({"operation": "event_chain"})],
            &json!({"synthetic": "final_liveness"}),
            "en",
        )
        .unwrap();
    }

    #[test]
    fn event_chain_plan_rejects_missing_empty_and_nonarray_event_lists() {
        let mut missing = three_chain_event_timeline();
        for row in &mut missing[1..=3] {
            let (resolved, _) = dispatched_values(&mut row.cause);
            resolved["input"].as_object_mut().unwrap().remove("events");
        }
        relink(&mut missing);
        assert_eq!(
            event_chain_error(&missing),
            "resolved event chain plan has no event list"
        );

        for (events, expected) in [
            (
                json!([]),
                "event chain exceeds its resolved event chain plan",
            ),
            (
                json!("not-an-array"),
                "resolved event chain plan has no event list",
            ),
            (Value::Null, "resolved event chain plan has no event list"),
        ] {
            let mut rows = three_chain_event_timeline();
            set_event_chain_plan(&mut rows, events);
            assert_eq!(event_chain_error(&rows), expected);
        }
    }

    #[test]
    fn event_chain_plan_rejects_too_many_too_few_and_wrong_events() {
        let mut too_many = three_chain_event_timeline();
        set_event_chain_plan(&mut too_many, json!(["first", "second"]));
        assert_eq!(
            event_chain_error(&too_many),
            "event chain exceeds its resolved event chain plan"
        );

        let mut too_few = three_chain_event_timeline();
        set_event_chain_plan(&mut too_few, json!(["first", "second", "third", "fourth"]));
        assert_eq!(
            event_chain_error(&too_few),
            "operation event chains do not cover its resolved event plan"
        );

        for (index, events, dispatched) in [
            (0, json!(["second", "first", "third"]), None),
            (1, json!(["first", "second", "third"]), Some(json!("wrong"))),
        ] {
            let mut rows = three_chain_event_timeline();
            set_event_chain_plan(&mut rows, events);
            if let Some(dispatched) = dispatched {
                let (_, event) = dispatched_values(&mut rows[index + 1].cause);
                *event = dispatched;
                relink(&mut rows);
            }
            assert_eq!(
                event_chain_error(&rows),
                "dispatched event does not match its resolved event chain plan"
            );
        }
    }

    #[test]
    fn event_chain_plan_rejects_a_plan_change_between_ordered_chains() {
        let mut rows = three_chain_event_timeline();
        let (resolved, _) = dispatched_values(&mut rows[2].cause);
        resolved["semantic_target"] = json!({"changed": true});
        relink(&mut rows);
        assert_eq!(
            event_chain_error(&rows),
            "one operation has inconsistent resolved event plans"
        );
    }

    #[test]
    fn click_plan_validation_keeps_its_exact_existing_contract() {
        let valid = resolved_multi("primary_click", &[json!("down"), json!("up")]);
        assert_eq!(
            validate_operation_plan_event(&valid, &json!("down"), 0).unwrap(),
            2
        );
        assert_eq!(
            validate_operation_plan_event(
                &json!({"input": {"kind": "primary_click"}}),
                &json!("down"),
                0,
            )
            .unwrap_err()
            .to_string(),
            "resolved click plan has no event list"
        );
        assert_eq!(
            validate_operation_plan_event(&valid, &json!("down"), 2)
                .unwrap_err()
                .to_string(),
            "event chain exceeds its resolved click plan"
        );
        assert_eq!(
            validate_operation_plan_event(&valid, &json!("wrong"), 1)
                .unwrap_err()
                .to_string(),
            "dispatched event does not match its resolved click plan"
        );
    }

    #[test]
    fn chain_rows_rejects_multiple_session_only_checkpoints() {
        let mut rows = complete_timeline();
        rows[1].boundary = TimelineBoundary::Session;
        rows[1].cause = TransitionCause::Session {
            requested: json!({"operation": 0}),
            resolved: resolved_single(json!("enter")),
            event: json!("enter"),
            handling: json!("ignored"),
        };
        rows[1].presentation = Presentation::Presented;
        relink(&mut rows);

        assert_eq!(
            validate_timeline(&rows, 1).unwrap_err().to_string(),
            "a session-only event chain has more than one checkpoint"
        );
    }

    #[test]
    fn chain_rows_rejects_a_non_host_user_continuation() {
        let mut rows = complete_timeline();
        rows[2].boundary = TimelineBoundary::Session;
        rows[2].cause = TransitionCause::Session {
            requested: json!({"operation": 0}),
            resolved: resolved_single(json!("enter")),
            event: json!("enter"),
            handling: json!("ignored"),
        };
        relink(&mut rows);

        assert_eq!(
            validate_timeline(&rows, 1).unwrap_err().to_string(),
            "a user event chain contains a non-host continuation"
        );
    }

    #[test]
    fn chain_rows_rejects_a_host_checkpoint_at_the_start() {
        let mut rows = complete_timeline();
        rows.remove(1);
        relink(&mut rows);

        assert_eq!(
            validate_timeline(&rows, 1).unwrap_err().to_string(),
            "an event chain must start with a session or user checkpoint"
        );
    }

    #[test]
    fn operation_plan_rejects_a_second_single_event_chain() {
        let mut rows = two_chain_click_timeline();
        for row in &mut rows[1..=2] {
            let (resolved, event) = dispatched_values(&mut row.cause);
            *resolved = resolved_single(json!("enter"));
            *event = json!("enter");
        }
        relink(&mut rows);

        assert_eq!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": "click"})],
                &json!({"synthetic": "final_liveness"}),
                "en",
            )
            .unwrap_err()
            .to_string(),
            "a single-event plan has more than one event chain"
        );
    }

    #[test]
    fn operation_plan_rejects_an_unsupported_input_kind() {
        let mut rows = complete_timeline();
        let (resolved, _) = dispatched_values(&mut rows[1].cause);
        *resolved = json!({
            "input": {"kind": "unsupported"},
            "semantic_target": null,
        });
        relink(&mut rows);

        assert_eq!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": 0})],
                &json!({"focus": true}),
                "en",
            )
            .unwrap_err()
            .to_string(),
            "resolved operation input has an unsupported event plan"
        );
    }

    #[test]
    fn liveness_event_rejects_a_chain_without_a_dispatch() {
        let cause = TransitionCause::Host {
            operation_index: None,
            round: 0,
            request: json!("none"),
            response: json!("none"),
            emitted: json!("none"),
        };

        assert_eq!(
            validate_liveness_event(&cause).unwrap_err().to_string(),
            "a liveness event chain has no dispatched event"
        );
    }

    #[test]
    fn liveness_event_rejects_an_event_outside_its_resolution() {
        let mut rows = complete_timeline();
        rows[3].cause = TransitionCause::Session {
            requested: json!({"focus": true}),
            resolved: json!({"event": "escape"}),
            event: json!("enter"),
            handling: json!("consumed"),
        };
        relink(&mut rows);

        assert_eq!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": 0})],
                &json!({"focus": true}),
                "en",
            )
            .unwrap_err()
            .to_string(),
            "liveness event does not match its resolved event"
        );
    }

    #[test]
    fn dispatch_events_rejects_an_incomplete_plan_before_the_next_operation() {
        let mut rows = two_chain_click_timeline();
        rows[2].operation_index = Some(1);
        rows[2].cause = TransitionCause::Reducer {
            requested: json!({"operation": "next"}),
            resolved: resolved_single(json!("enter")),
            event: json!("enter"),
            action: json!("activate"),
            emitted: json!("none"),
        };
        relink(&mut rows);

        assert_eq!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": "click"}), json!({"operation": "next"}),],
                &json!({"synthetic": "final_liveness"}),
                "en",
            )
            .unwrap_err()
            .to_string(),
            "operation event chains do not cover its resolved event plan"
        );
    }

    #[test]
    fn dispatch_events_rejects_an_operation_chain_without_a_dispatch() {
        let mut rows = complete_timeline();
        rows[1].boundary = TimelineBoundary::HostAction;
        rows[1].cause = TransitionCause::Host {
            operation_index: Some(0),
            round: 0,
            request: json!("open"),
            response: json!("opened"),
            emitted: json!("none"),
        };
        relink(&mut rows);

        assert_eq!(
            validate_dispatch_events(&rows).unwrap_err().to_string(),
            "an operation event chain has no dispatched event"
        );
    }

    #[test]
    fn dispatch_events_rejects_changed_plans_within_one_operation() {
        let mut rows = two_chain_click_timeline();
        rows[2].cause = TransitionCause::Reducer {
            requested: json!({"operation": "click"}),
            resolved: json!({
                "input": {
                    "kind": "primary_click",
                    "events": ["mouse_down", "mouse_up"],
                },
                "semantic_target": {"changed": true},
            }),
            event: json!("mouse_up"),
            action: json!("activate"),
            emitted: json!("none"),
        };
        relink(&mut rows);

        assert_eq!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": "click"})],
                &json!({"synthetic": "final_liveness"}),
                "en",
            )
            .unwrap_err()
            .to_string(),
            "one operation has inconsistent resolved event plans"
        );
    }

    #[test]
    fn dispatch_events_rejects_an_incomplete_plan_at_the_timeline_tail() {
        let mut rows = two_chain_click_timeline();
        rows.truncate(2);
        relink(&mut rows);

        assert_eq!(
            validate_dispatch_events(&rows).unwrap_err().to_string(),
            "operation event chains do not cover its resolved event plan"
        );
    }

    #[test]
    fn timeline_accepts_two_dispatch_chains_for_one_click_operation() {
        let rows = two_chain_click_timeline();
        validate_timeline_semantics(
            &rows,
            &[json!({"operation": "click"})],
            &json!({"synthetic": "final_liveness"}),
            "en",
        )
        .unwrap();

        let mut local_rows = rows;
        for row in &mut local_rows[1..=2] {
            let (resolved, _) = dispatched_values(&mut row.cause);
            resolved["input"]["kind"] = json!("local_primary_click");
        }
        relink(&mut local_rows);
        validate_timeline_semantics(
            &local_rows,
            &[json!({"operation": "click"})],
            &json!({"synthetic": "final_liveness"}),
            "en",
        )
        .unwrap();
    }

    #[test]
    fn timeline_rejects_duplicate_gapped_and_mixed_event_chains() {
        let rows = two_chain_click_timeline();

        let mut duplicate = rows.clone();
        let final_liveness = duplicate.pop().unwrap();
        let mut third_chain = duplicate[1].clone();
        third_chain.event_chain.as_mut().unwrap().sequence = 2;
        duplicate.push(third_chain);
        duplicate.push(final_liveness);
        relink(&mut duplicate);
        validate_timeline(&duplicate, 1).unwrap();
        duplicate[3].event_chain.as_mut().unwrap().sequence = 0;
        relink(&mut duplicate);
        assert!(validate_timeline(&duplicate, 1).is_err());

        let mut gap = rows.clone();
        gap[2].event_chain.as_mut().unwrap().sequence = 2;
        relink(&mut gap);
        assert!(validate_timeline(&gap, 1).is_err());

        let mut mixed_operation = complete_timeline();
        mixed_operation[2].operation_index = Some(1);
        if let TransitionCause::Host {
            operation_index, ..
        } = &mut mixed_operation[2].cause
        {
            *operation_index = Some(1);
        }
        relink(&mut mixed_operation);
        assert!(validate_timeline(&mixed_operation, 2).is_err());

        let mut mixed_phase = complete_timeline();
        mixed_phase[2].phase = TimelinePhase::FinalLiveness;
        relink(&mut mixed_phase);
        assert!(validate_timeline(&mixed_phase, 1).is_err());
    }

    #[test]
    fn timeline_semantics_rejects_an_event_outside_its_resolved_plan() {
        let mut rows = two_chain_click_timeline();
        let (_, event) = dispatched_values(&mut rows[2].cause);
        *event = json!("mouse_down");
        relink(&mut rows);
        assert!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": "click"})],
                &json!({"synthetic": "final_liveness"}),
                "en",
            )
            .is_err()
        );

        let mut incomplete = two_chain_click_timeline();
        incomplete.remove(2);
        relink(&mut incomplete);
        assert!(
            validate_timeline_semantics(
                &incomplete,
                &[json!({"operation": "click"})],
                &json!({"synthetic": "final_liveness"}),
                "en",
            )
            .is_err()
        );

        let mut wrong_single = complete_timeline();
        let (_, event) = dispatched_values(&mut wrong_single[1].cause);
        *event = json!("space");
        relink(&mut wrong_single);
        assert!(
            validate_timeline_semantics(
                &wrong_single,
                &[json!({"operation": 0})],
                &json!({"focus": true}),
                "en",
            )
            .is_err()
        );

        let mut wrong_liveness = complete_timeline();
        let (_, event) = dispatched_values(&mut wrong_liveness[3].cause);
        *event = json!({"synthetic": "not_applicable"});
        relink(&mut wrong_liveness);
        assert!(
            validate_timeline_semantics(
                &wrong_liveness,
                &[json!({"operation": 0})],
                &json!({"focus": true}),
                "en",
            )
            .is_err()
        );
    }

    #[test]
    fn timeline_semantics_requires_the_noop_synthetic_event() {
        let mut rows = complete_timeline();
        rows.remove(2);
        rows[1].boundary = TimelineBoundary::Session;
        rows[1].cause = TransitionCause::Session {
            requested: json!({"operation": "noop"}),
            resolved: json!({
                "input": {"kind": "not_applicable"},
                "semantic_target": null,
            }),
            event: json!({"synthetic": "not_applicable"}),
            handling: json!("not_applicable"),
        };
        rows[1].presentation = Presentation::Presented;
        relink(&mut rows);
        validate_timeline_semantics(
            &rows,
            &[json!({"operation": "noop"})],
            &json!({"focus": true}),
            "en",
        )
        .unwrap();

        let (_, event) = dispatched_values(&mut rows[1].cause);
        *event = json!({"synthetic": "already_terminal"});
        relink(&mut rows);
        assert!(
            validate_timeline_semantics(
                &rows,
                &[json!({"operation": "noop"})],
                &json!({"focus": true}),
                "en",
            )
            .is_err()
        );
    }

    #[test]
    fn timeline_requires_complete_ordered_operations_and_passed_terminal_liveness() {
        let rows = complete_timeline();
        validate_timeline(&rows, 1).unwrap();
        assert!(validate_timeline(&[], 1).is_err());

        let mut invalid_initial = rows.clone();
        invalid_initial[0].phase = TimelinePhase::FinalLiveness;
        assert!(validate_timeline(&invalid_initial, 1).is_err());

        let mut bad_schema = rows.clone();
        bad_schema[1].schema = 1;
        relink(&mut bad_schema);
        assert!(validate_timeline(&bad_schema, 1).is_err());

        let mut out_of_range = rows.clone();
        out_of_range[1].operation_index = Some(1);
        relink(&mut out_of_range);
        assert!(validate_timeline(&out_of_range, 1).is_err());

        let mut two_operations = rows.clone();
        let final_liveness = two_operations.pop().unwrap();
        let mut second_operation = row(0, None);
        second_operation.operation_index = Some(1);
        second_operation.event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 1,
        });
        second_operation.boundary = TimelineBoundary::Session;
        second_operation.cause = TransitionCause::Session {
            requested: json!({"operation": 1}),
            resolved: resolved_single(json!("focus_gained")),
            event: json!("focus_gained"),
            handling: json!("ignored"),
        };
        two_operations.push(second_operation);
        two_operations.push(final_liveness);
        relink(&mut two_operations);
        validate_timeline(&two_operations, 2).unwrap();

        let mut resumed = rows.clone();
        resumed.last_mut().unwrap().liveness = None;
        let mut operation_after_liveness = row(0, None);
        operation_after_liveness.operation_index = Some(0);
        let final_liveness = rows.last().unwrap().clone();
        resumed.push(operation_after_liveness);
        resumed.push(final_liveness);
        relink(&mut resumed);
        assert!(validate_timeline(&resumed, 1).is_err());

        let mut host_index = rows.clone();
        if let TransitionCause::Host {
            operation_index, ..
        } = &mut host_index[2].cause
        {
            *operation_index = None;
        }
        relink(&mut host_index);
        assert!(validate_timeline(&host_index, 1).is_err());

        let mut object_kind = rows.clone();
        object_kind[1].reducer.kind = ObjectKind::Host;
        relink(&mut object_kind);
        assert!(validate_timeline(&object_kind, 1).is_err());

        let mut missing_operation = rows.clone();
        assert!(validate_timeline(&missing_operation, 2).is_err());
        missing_operation[1].operation_index = Some(1);
        missing_operation[2].operation_index = Some(1);
        missing_operation[1].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 1,
        });
        missing_operation[2].event_chain = missing_operation[1].event_chain;
        if let TransitionCause::Host {
            operation_index, ..
        } = &mut missing_operation[2].cause
        {
            *operation_index = Some(1);
        }
        relink(&mut missing_operation);
        assert!(validate_timeline(&missing_operation, 2).is_err());

        let mut failed = rows.clone();
        failed.last_mut().unwrap().liveness = Some(LivenessResult::Failed {
            message: "did not settle".to_owned(),
        });
        relink(&mut failed);
        assert!(validate_timeline(&failed, 1).is_err());

        let mut no_result = rows.clone();
        no_result.last_mut().unwrap().liveness = None;
        relink(&mut no_result);
        assert!(validate_timeline(&no_result, 1).is_err());

        let mut repeated_initial = rows;
        repeated_initial.last_mut().unwrap().boundary = TimelineBoundary::Initial;
        repeated_initial.last_mut().unwrap().cause = TransitionCause::Initial;
        relink(&mut repeated_initial);
        assert!(validate_timeline(&repeated_initial, 1).is_err());
    }

    #[test]
    fn timeline_enforces_user_host_rounds_and_synthetic_chain_identity() {
        let rows = complete_timeline();

        let mut host_without_user = rows.clone();
        host_without_user[1].boundary = TimelineBoundary::HostAction;
        host_without_user[1].cause = TransitionCause::Host {
            operation_index: Some(0),
            round: 0,
            request: json!("request"),
            response: json!("response"),
            emitted: json!("none"),
        };
        relink(&mut host_without_user);
        assert!(validate_timeline(&host_without_user, 1).is_err());

        let mut skipped_round = rows.clone();
        if let TransitionCause::Host { round, .. } = &mut skipped_round[2].cause {
            *round = 1;
        }
        relink(&mut skipped_round);
        assert!(validate_timeline(&skipped_round, 1).is_err());

        let mut repeated_session = rows.clone();
        for row in &mut repeated_session[1..=2] {
            row.boundary = TimelineBoundary::Session;
            row.cause = TransitionCause::Session {
                requested: json!("focus"),
                resolved: resolved_single(json!("focus")),
                event: json!("focus"),
                handling: json!("ignored"),
            };
        }
        relink(&mut repeated_session);
        assert!(validate_timeline(&repeated_session, 1).is_err());

        let mut non_host_continuation = rows.clone();
        non_host_continuation[2].boundary = TimelineBoundary::UserAction;
        non_host_continuation[2].cause = non_host_continuation[1].cause.clone();
        relink(&mut non_host_continuation);
        assert!(validate_timeline(&non_host_continuation, 1).is_err());

        let mut wrong_phase = rows.clone();
        wrong_phase[1].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::FinalLiveness,
            sequence: 0,
        });
        relink(&mut wrong_phase);
        assert!(validate_timeline(&wrong_phase, 1).is_err());

        let mut wrong_operation_chain = rows.clone();
        wrong_operation_chain[1].event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 1,
        });
        relink(&mut wrong_operation_chain);
        assert!(validate_timeline(&wrong_operation_chain, 1).is_err());

        let mut duplicate_round = rows.clone();
        let final_liveness = duplicate_round.pop().unwrap();
        let mut repeated_host = duplicate_round.last().unwrap().clone();
        if let TransitionCause::Host { round, .. } = &mut repeated_host.cause {
            *round = 0;
        }
        duplicate_round.push(repeated_host);
        duplicate_round.push(final_liveness);
        relink(&mut duplicate_round);
        assert!(validate_timeline(&duplicate_round, 1).is_err());

        let mut final_host_chain = rows.clone();
        final_host_chain.pop();
        let mut synthetic_user = row(0, None);
        synthetic_user.phase = TimelinePhase::FinalLiveness;
        synthetic_user.event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::FinalLiveness,
            sequence: 0,
        });
        synthetic_user.operation_index = None;
        synthetic_user.boundary = TimelineBoundary::UserAction;
        synthetic_user.cause = TransitionCause::Reducer {
            requested: json!({"synthetic": "escape"}),
            resolved: json!({"event": "escape"}),
            event: json!("escape"),
            action: json!("back"),
            emitted: json!("reload"),
        };
        synthetic_user.presentation = Presentation::NotPresented;
        let mut synthetic_host = synthetic_user.clone();
        synthetic_host.boundary = TimelineBoundary::HostAction;
        synthetic_host.cause = TransitionCause::Host {
            operation_index: None,
            round: 0,
            request: json!("reload"),
            response: json!("surface"),
            emitted: json!("none"),
        };
        synthetic_host.presentation = Presentation::Presented;
        synthetic_host.liveness = Some(LivenessResult::Passed);
        final_host_chain.push(synthetic_user);
        final_host_chain.push(synthetic_host);
        relink(&mut final_host_chain);
        validate_timeline(&final_host_chain, 1).unwrap();

        let mut bad_final_round = final_host_chain;
        if let TransitionCause::Host { round, .. } = &mut bad_final_round.last_mut().unwrap().cause
        {
            *round = 1;
        }
        relink(&mut bad_final_round);
        assert!(validate_timeline(&bad_final_round, 1).is_err());

        let mut missing_chain = rows.clone();
        missing_chain.last_mut().unwrap().event_chain = None;
        relink(&mut missing_chain);
        assert!(validate_timeline(&missing_chain, 1).is_err());

        let mut wrong_synthetic_chain = rows;
        wrong_synthetic_chain.last_mut().unwrap().event_chain = Some(EventChainIdentity {
            phase: TimelinePhase::FinalLiveness,
            sequence: 1,
        });
        relink(&mut wrong_synthetic_chain);
        assert!(validate_timeline(&wrong_synthetic_chain, 1).is_err());
    }

    #[test]
    fn timeline_allows_resize_viewports_and_defers_locale_rules_to_semantics() {
        let mut resized = complete_timeline();
        resized[2].viewport.width = 24;
        resized[2].viewport.height = 6;
        resized[3].viewport = resized[2].viewport;
        relink(&mut resized);
        validate_timeline(&resized, 1).unwrap();

        resized[2].locale = "zh-CN".to_owned();
        relink(&mut resized);
        validate_timeline(&resized, 1).unwrap();
        assert!(
            validate_timeline_semantics(
                &resized,
                &[json!({"operation": 0})],
                &json!({"focus": true}),
                "en",
            )
            .is_err()
        );
    }

    #[test]
    fn timeline_semantics_bind_requested_operations_and_effect_chains() {
        let rows = complete_timeline();
        let operations = vec![json!({"operation": 0})];
        let final_liveness = json!({"focus": true});
        validate_timeline_semantics(&rows, &operations, &final_liveness, "en").unwrap();

        let mut wrong_operation = rows.clone();
        if let TransitionCause::Reducer { requested, .. } = &mut wrong_operation[1].cause {
            *requested = json!({"operation": 9});
        }
        relink(&mut wrong_operation);
        assert!(
            validate_timeline_semantics(&wrong_operation, &operations, &final_liveness, "en")
                .is_err()
        );

        let mut wrong_liveness = rows.clone();
        if let TransitionCause::Session { requested, .. } = &mut wrong_liveness[3].cause {
            *requested = json!({"focus": false});
        }
        relink(&mut wrong_liveness);
        assert!(
            validate_timeline_semantics(&wrong_liveness, &operations, &final_liveness, "en")
                .is_err()
        );

        let mut wrong_host_request = rows;
        if let TransitionCause::Host { request, .. } = &mut wrong_host_request[2].cause {
            *request = json!({"open": false});
        }
        relink(&mut wrong_host_request);
        assert!(
            validate_timeline_semantics(&wrong_host_request, &operations, &final_liveness, "en")
                .is_err()
        );

        let mut two_host_rounds = complete_timeline();
        let final_row = two_host_rounds.pop().unwrap();
        let mut second_host = two_host_rounds.last().unwrap().clone();
        second_host.cause = TransitionCause::Host {
            operation_index: Some(0),
            round: 1,
            request: json!("none"),
            response: json!({"saved": true}),
            emitted: json!({"done": true}),
        };
        second_host.presentation = Presentation::NotPresented;
        two_host_rounds.push(second_host);
        two_host_rounds.push(final_row);
        relink(&mut two_host_rounds);
        validate_timeline_semantics(&two_host_rounds, &operations, &final_liveness, "en").unwrap();
        if let TransitionCause::Host { request, .. } = &mut two_host_rounds[3].cause {
            *request = json!({"unrelated": true});
        }
        relink(&mut two_host_rounds);
        assert!(
            validate_timeline_semantics(&two_host_rounds, &operations, &final_liveness, "en")
                .is_err()
        );

        let mut final_reducer = complete_timeline();
        let final_row = final_reducer.last_mut().unwrap();
        final_row.boundary = TimelineBoundary::UserAction;
        final_row.cause = TransitionCause::Reducer {
            requested: final_liveness.clone(),
            resolved: json!({"event": "escape"}),
            event: json!("escape"),
            action: json!("back"),
            emitted: json!("none"),
        };
        relink(&mut final_reducer);
        validate_timeline_semantics(&final_reducer, &operations, &final_liveness, "en").unwrap();

        let mut operation_session = complete_timeline();
        operation_session.remove(2);
        operation_session[1].boundary = TimelineBoundary::Session;
        operation_session[1].cause = TransitionCause::Session {
            requested: operations[0].clone(),
            resolved: resolved_single(json!("focus_gained")),
            event: json!("focus_gained"),
            handling: json!("ignored"),
        };
        operation_session[1].presentation = Presentation::Presented;
        relink(&mut operation_session);
        validate_timeline_semantics(&operation_session, &operations, &final_liveness, "en")
            .unwrap();
    }

    #[test]
    fn timeline_locale_changes_only_from_a_matching_host_response() {
        let operations = vec![json!({"operation": 0})];
        let final_liveness = json!({"focus": true});
        let mut rows = complete_timeline();
        rows[2].locale = "zh-CN".to_owned();
        rows[3].locale = "zh-CN".to_owned();
        if let TransitionCause::Host { response, .. } = &mut rows[2].cause {
            *response = json!({"preferences_loaded": [{"locale": "zh-CN"}]});
        }
        relink(&mut rows);
        validate_timeline_semantics(&rows, &operations, &final_liveness, "en").unwrap();

        let mut wrong_initial = rows.clone();
        wrong_initial[0].locale = "zh-CN".to_owned();
        relink(&mut wrong_initial);
        assert!(
            validate_timeline_semantics(&wrong_initial, &operations, &final_liveness, "en")
                .is_err()
        );

        let mut wrong_response = rows.clone();
        if let TransitionCause::Host { response, .. } = &mut wrong_response[2].cause {
            *response = json!({"preferences_loaded": {"locale": "zh-TW"}});
        }
        relink(&mut wrong_response);
        assert!(
            validate_timeline_semantics(&wrong_response, &operations, &final_liveness, "en")
                .is_err()
        );

        let mut changed_on_session = complete_timeline();
        changed_on_session[3].locale = "zh-CN".to_owned();
        relink(&mut changed_on_session);
        assert!(
            validate_timeline_semantics(&changed_on_session, &operations, &final_liveness, "en")
                .is_err()
        );
    }

    #[test]
    fn asciicast_v3_validation_checks_header_and_event_shapes() {
        let valid = concat!(
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n",
            "[0.0,\"o\",\"screen\"]\n",
            "[0.25,\"r\",\"120x30\"]\n"
        );
        validate_asciicast_v3(valid.as_bytes()).unwrap();

        for invalid in [
            "",
            "{\"version\":2,\"term\":{\"cols\":80,\"rows\":24}}\n",
            "{\"version\":3,\"term\":{\"cols\":0,\"rows\":24}}\n",
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n[-1,\"o\",\"x\"]\n",
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n[0,\"i\",\"x\"]\n",
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n[0,\"o\",3]\n",
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n[]\n",
            "{\"version\":3,\"term\":{\"cols\":80.5,\"rows\":24}}\n",
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n\n[0,\"o\",\"x\"]\n",
            "{\"version\":3,\"term\":{\"cols\":80,\"rows\":24}}\n[0,\"o\",\"x\",\"extra\"]\n",
        ] {
            assert!(validate_asciicast_v3(invalid.as_bytes()).is_err());
        }
    }

    fn chunk(profile: &SafeProfileId, id: &str, start: u32, end: u32) -> ChunkDescriptor {
        ChunkDescriptor {
            id: id.to_owned(),
            profile: profile.clone(),
            start_sequence: start,
            end_sequence: end,
            row_count: end - start,
            sha256: sha256_hex(id.as_bytes()),
            first_row_sha256: ZERO_DIGEST.to_owned(),
            last_row_sha256: ZERO_DIGEST.to_owned(),
            first_chain: None,
            last_chain: Some(EventChainIdentity {
                phase: TimelinePhase::Operations,
                sequence: 0,
            }),
            continues_previous_chain: false,
            continues_next_chain: false,
        }
    }

    #[test]
    fn chunks_are_built_from_rows_without_splitting_one_operation_chain() {
        let requested_profile = profile("en-80x24");
        let rows = complete_timeline();
        let chunks = build_chunks(&requested_profile, &rows, 2).unwrap();
        validate_chunks(&requested_profile, &rows, &chunks, 2).unwrap();
        assert!(validate_chunks(&requested_profile, &rows, &chunks, 1).is_err());
        assert!(chunks.iter().all(|chunk| chunk.row_count <= 2));
        assert!(chunks[0].continues_next_chain);
        assert!(chunks[1].continues_previous_chain);
        assert_eq!(chunks[0].last_chain, chunks[1].first_chain);
        assert!(build_chunks(&requested_profile, &[], 2).is_err());
        assert!(build_chunks(&requested_profile, &rows, 0).is_err());
        assert_eq!(
            validate_chunks(&requested_profile, &[], &chunks, 2)
                .unwrap_err()
                .to_string(),
            "rows and chunks must be nonempty"
        );
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &[], 2)
                .unwrap_err()
                .to_string(),
            "rows and chunks must be nonempty"
        );
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &chunks, 0)
                .unwrap_err()
                .to_string(),
            "rows and chunks must be nonempty"
        );

        let mut wrong_profile_rows = rows.clone();
        wrong_profile_rows[0].profile = profile("other");
        assert!(build_chunks(&requested_profile, &wrong_profile_rows, 2).is_err());
        assert!(validate_chunks(&requested_profile, &wrong_profile_rows, &chunks, 2).is_err());

        let mut wrong_chunk_profile = chunks.clone();
        wrong_chunk_profile[0].profile = profile("other");
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &wrong_chunk_profile, 2)
                .unwrap_err()
                .to_string(),
            "chunk profile or id is invalid"
        );

        let mut gap = chunks.clone();
        gap[0].start_sequence = 1;
        gap[0].row_count = 1;
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &gap, 2)
                .unwrap_err()
                .to_string(),
            "chunks have a gap, overlap, or invalid row count"
        );

        let mut empty_range = chunks.clone();
        empty_range[0].end_sequence = 0;
        empty_range[0].row_count = 0;
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &empty_range, 2)
                .unwrap_err()
                .to_string(),
            "chunks have a gap, overlap, or invalid row count"
        );

        let mut wrong_row_count = chunks.clone();
        wrong_row_count[0].row_count += 1;
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &wrong_row_count, 2)
                .unwrap_err()
                .to_string(),
            "chunks have a gap, overlap, or invalid row count"
        );

        let mut duplicate_id = chunks.clone();
        duplicate_id[1].id = duplicate_id[0].id.clone();
        assert_eq!(
            validate_chunks(&requested_profile, &rows, &duplicate_id, 2)
                .unwrap_err()
                .to_string(),
            "chunk profile or id is invalid"
        );

        let mut after_timeline = chunks.clone();
        after_timeline[1].end_sequence += 1;
        after_timeline[1].row_count += 1;
        assert!(validate_chunks(&requested_profile, &rows, &after_timeline, 3).is_err());

        let mut incomplete = chunks.clone();
        incomplete.pop();
        assert!(validate_chunks(&requested_profile, &rows, &incomplete, 2).is_err());

        let mut corrupt_digest = chunks;
        corrupt_digest[0].last_row_sha256 = ZERO_DIGEST.to_owned();
        assert!(validate_chunks(&requested_profile, &rows, &corrupt_digest, 2).is_err());

        let mut overlap = build_chunks(&requested_profile, &rows, 2).unwrap();
        overlap[1].start_sequence = overlap[1].start_sequence.saturating_sub(1);
        assert!(validate_chunks(&requested_profile, &rows, &overlap, 2).is_err());

        let mut broken_predecessor = rows.clone();
        let descriptors = build_chunks(&requested_profile, &broken_predecessor, 2).unwrap();
        broken_predecessor[2].previous_row_sha256 = Some(ZERO_DIGEST.to_owned());
        assert!(build_chunks(&requested_profile, &broken_predecessor, 2).is_err());
        assert!(validate_chunks(&requested_profile, &broken_predecessor, &descriptors, 2).is_err());

        let mut long_chain = complete_timeline();
        let final_liveness = long_chain.pop().unwrap();
        for round in 1..=2 {
            let mut host = long_chain.last().unwrap().clone();
            if let TransitionCause::Host {
                round: host_round, ..
            } = &mut host.cause
            {
                *host_round = round;
            }
            long_chain.push(host);
        }
        long_chain.push(final_liveness);
        relink(&mut long_chain);
        validate_timeline(&long_chain, 1).unwrap();
        let long_chunks = build_chunks(&requested_profile, &long_chain, 2).unwrap();
        assert!(long_chunks.iter().all(|chunk| chunk.row_count <= 2));
        assert!(long_chunks[1].continues_previous_chain);
        assert!(long_chunks[1].continues_next_chain);
        validate_chunks(&requested_profile, &long_chain, &long_chunks, 2).unwrap();
    }

    #[test]
    fn chunks_can_split_between_two_click_event_chains() {
        let profile = profile("en-80x24");
        let rows = two_chain_click_timeline();
        let chunks = build_chunks(&profile, &rows, 2).unwrap();

        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].start_sequence, 0);
        assert_eq!(chunks[0].end_sequence, 2);
        assert_eq!(chunks[0].last_chain.unwrap().sequence, 0);
        assert_eq!(chunks[1].start_sequence, 2);
        assert_eq!(chunks[1].first_chain.unwrap().sequence, 1);
        assert!(!chunks[0].continues_next_chain);
        assert!(!chunks[1].continues_previous_chain);
        validate_chunks(&profile, &rows, &chunks, 2).unwrap();
    }

    fn manifest() -> CorpusManifest {
        let operations = sha256_hex(b"operations");
        let profiles = [
            ("en-80x24", "en", 80, 24),
            ("zh-cn-120x30", "zh-CN", 120, 30),
            ("zh-tw-40x40", "zh-TW", 40, 40),
            ("pseudo-120x12", "x-pseudo", 120, 12),
        ];
        CorpusManifest {
            schema: 1,
            operations_sha256: operations.clone(),
            profiles: profiles
                .into_iter()
                .map(|(id, locale, width, height)| {
                    let profile = profile(id);
                    ProfileManifest {
                        id: profile.clone(),
                        initial_locale: locale.to_owned(),
                        initial_viewport: RectSnapshot {
                            x: 0,
                            y: 0,
                            width: Size::new(width, height).width,
                            height,
                        },
                        operations_sha256: operations.clone(),
                        cast_sha256: sha256_hex(format!("cast-{id}").as_bytes()),
                        cast_byte_size: 100,
                        row_count: 1,
                        maximum_chunk_rows: 1,
                        chunks: vec![chunk(&profile, &format!("{id}-a"), 0, 1)],
                    }
                })
                .collect(),
        }
    }

    #[test]
    fn every_profile_bearing_contract_rejects_raw_invalid_text() {
        let mut timeline: Value = serde_json::to_value(row(0, None)).unwrap();
        timeline["profile"] = json!("Upper");
        assert!(serde_json::from_value::<TimelineRow>(timeline).is_err());

        let mut descriptor: Value =
            serde_json::to_value(chunk(&profile("en-80x24"), "chunk", 0, 1)).unwrap();
        descriptor["profile"] = json!("Upper");
        assert!(serde_json::from_value::<ChunkDescriptor>(descriptor).is_err());

        let mut profile: Value = serde_json::to_value(&manifest().profiles[0]).unwrap();
        profile["id"] = json!("Upper");
        assert!(serde_json::from_value::<ProfileManifest>(profile).is_err());

        let progress = json!({
            "manifest_sha256": ZERO_DIGEST,
            "complete": false,
            "review_sha256": null,
            "reviewed_chunks": {},
            "profile_verdicts": {"Upper": "no_findings"},
        });
        assert!(serde_json::from_value::<ReviewProgress>(progress).is_err());
    }

    #[test]
    fn manifest_requires_the_exact_four_nonempty_unique_profiles() {
        let valid = manifest();
        validate_manifest(&valid).unwrap();

        let mut schema = valid.clone();
        schema.schema = 2;
        assert!(validate_manifest(&schema).is_err());

        let mut missing = valid.clone();
        missing.profiles.pop();
        assert!(validate_manifest(&missing).is_err());
        let incomplete_progress = ReviewProgress {
            manifest_sha256: manifest_digest(&missing).unwrap(),
            complete: true,
            review_sha256: Some(sha256_hex(b"review")),
            reviewed_chunks: missing
                .profiles
                .iter()
                .flat_map(|profile| &profile.chunks)
                .map(|chunk| (chunk.id.clone(), chunk.sha256.clone()))
                .collect(),
            profile_verdicts: missing
                .profiles
                .iter()
                .map(|profile| (profile.id.clone(), ReviewVerdict::NoFindings))
                .collect(),
        };
        assert!(validate_progress(&missing, &incomplete_progress).is_err());

        let mut duplicate = valid.clone();
        duplicate.profiles[3] = duplicate.profiles[0].clone();
        assert!(validate_manifest(&duplicate).is_err());

        let mut empty = valid.clone();
        empty.profiles[0].row_count = 0;
        empty.profiles[0].chunks.clear();
        assert!(validate_manifest(&empty).is_err());

        let mut metadata = valid.clone();
        metadata.profiles[0].initial_locale = "zh-CN".to_owned();
        assert!(validate_manifest(&metadata).is_err());

        let mut chunk_profile = valid.clone();
        chunk_profile.profiles[0].chunks[0].profile = profile("other");
        assert!(validate_manifest(&chunk_profile).is_err());

        let mut chunk_gap = valid.clone();
        chunk_gap.profiles[0].chunks[0].start_sequence = 1;
        assert!(validate_manifest(&chunk_gap).is_err());

        let mut row_count = valid.clone();
        row_count.profiles[0].row_count = 2;
        assert!(validate_manifest(&row_count).is_err());

        let mut zero_limit = valid.clone();
        zero_limit.profiles[0].maximum_chunk_rows = 0;
        assert!(validate_manifest(&zero_limit).is_err());

        let mut false_continuation = valid.clone();
        false_continuation.profiles[0].chunks[0].continues_previous_chain = true;
        assert!(validate_manifest(&false_continuation).is_err());

        let mut mismatched_continuation = valid.clone();
        let profile = &mut mismatched_continuation.profiles[0];
        profile.row_count = 2;
        let mut first = chunk(&profile.id, "first", 0, 1);
        first.last_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 0,
        });
        first.continues_next_chain = true;
        let mut second = chunk(&profile.id, "second", 1, 2);
        second.first_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 1,
        });
        second.last_chain = second.first_chain;
        second.continues_previous_chain = true;
        profile.chunks = vec![first, second];
        assert!(validate_manifest(&mismatched_continuation).is_err());

        let mut unmarked_continuation = valid.clone();
        let profile = &mut unmarked_continuation.profiles[0];
        profile.row_count = 2;
        let mut first = chunk(&profile.id, "same-first", 0, 1);
        first.last_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 0,
        });
        let mut second = chunk(&profile.id, "same-second", 1, 2);
        second.first_chain = first.last_chain;
        second.last_chain = first.last_chain;
        profile.chunks = vec![first, second];
        assert!(validate_manifest(&unmarked_continuation).is_err());

        let mut unmarked_previous = valid.clone();
        let profile = &mut unmarked_previous.profiles[0];
        profile.row_count = 2;
        let mut first = chunk(&profile.id, "same-first", 0, 1);
        first.last_chain = Some(EventChainIdentity {
            phase: TimelinePhase::Operations,
            sequence: 0,
        });
        first.continues_next_chain = true;
        let mut second = chunk(&profile.id, "same-second", 1, 2);
        second.first_chain = first.last_chain;
        second.last_chain = first.last_chain;
        profile.chunks = vec![first, second];
        assert!(validate_manifest(&unmarked_previous).is_err());

        let mut duplicate_chunk = valid.clone();
        duplicate_chunk.profiles[1].chunks[0].id = duplicate_chunk.profiles[0].chunks[0].id.clone();
        assert!(validate_manifest(&duplicate_chunk).is_err());

        let mut operations = valid;
        operations.operations_sha256.clear();
        assert!(validate_manifest(&operations).is_err());

        let mut cast_digest = manifest();
        cast_digest.profiles[0].cast_sha256.clear();
        assert!(validate_manifest(&cast_digest).is_err());

        let mut cast_size = manifest();
        cast_size.profiles[0].cast_byte_size = 0;
        assert!(validate_manifest(&cast_size).is_err());
    }

    #[test]
    fn progress_is_pinned_to_manifest_and_chunk_digests() {
        let manifest = manifest();
        let manifest_sha256 = manifest_digest(&manifest).unwrap();
        let first_chunk = &manifest.profiles[0].chunks[0];
        let pending = ReviewProgress {
            manifest_sha256: manifest_sha256.clone(),
            complete: false,
            review_sha256: None,
            reviewed_chunks: BTreeMap::from([(first_chunk.id.clone(), first_chunk.sha256.clone())]),
            profile_verdicts: BTreeMap::new(),
        };
        validate_progress(&manifest, &pending).unwrap();

        let mut wrong_manifest = pending.clone();
        wrong_manifest.manifest_sha256 = ZERO_DIGEST.to_owned();
        assert!(validate_progress(&manifest, &wrong_manifest).is_err());

        let mut premature = pending.clone();
        premature.complete = true;
        assert!(validate_progress(&manifest, &premature).is_err());

        let stale = ReviewProgress {
            manifest_sha256,
            complete: false,
            review_sha256: None,
            reviewed_chunks: BTreeMap::from([(first_chunk.id.clone(), ZERO_DIGEST.to_owned())]),
            profile_verdicts: BTreeMap::new(),
        };
        assert!(validate_progress(&manifest, &stale).is_err());
    }

    #[test]
    fn progress_accepts_completion_only_after_every_declared_chunk() {
        let manifest = manifest();
        let reviewed_chunks = manifest
            .profiles
            .iter()
            .flat_map(|profile| &profile.chunks)
            .map(|chunk| (chunk.id.clone(), chunk.sha256.clone()))
            .collect();
        let progress = ReviewProgress {
            manifest_sha256: manifest_digest(&manifest).unwrap(),
            complete: true,
            review_sha256: Some(sha256_hex(b"review report")),
            reviewed_chunks,
            profile_verdicts: manifest
                .profiles
                .iter()
                .map(|profile| (profile.id.clone(), ReviewVerdict::NoFindings))
                .collect(),
        };
        validate_progress(&manifest, &progress).unwrap();

        let mut missing_review = progress.clone();
        missing_review.review_sha256 = None;
        assert!(validate_progress(&manifest, &missing_review).is_err());

        let mut invalid_review = progress.clone();
        invalid_review.review_sha256 = Some("invalid".to_owned());
        assert!(validate_progress(&manifest, &invalid_review).is_err());

        let mut missing_verdict = progress.clone();
        missing_verdict.profile_verdicts.pop_first();
        assert!(validate_progress(&manifest, &missing_verdict).is_err());

        let mut unknown_profile = progress;
        unknown_profile
            .profile_verdicts
            .insert(profile("other"), ReviewVerdict::FindingsInReport);
        assert!(validate_progress(&manifest, &unknown_profile).is_err());

        let mut findings = manifest
            .profiles
            .iter()
            .map(|profile| (profile.id.clone(), ReviewVerdict::NoFindings))
            .collect::<BTreeMap<_, _>>();
        findings.insert(
            manifest.profiles[0].id.clone(),
            ReviewVerdict::FindingsInReport,
        );
        let completed_with_findings = ReviewProgress {
            manifest_sha256: manifest_digest(&manifest).unwrap(),
            complete: true,
            review_sha256: Some(sha256_hex(b"review with findings")),
            reviewed_chunks: manifest
                .profiles
                .iter()
                .flat_map(|profile| &profile.chunks)
                .map(|chunk| (chunk.id.clone(), chunk.sha256.clone()))
                .collect(),
            profile_verdicts: findings,
        };
        validate_progress(&manifest, &completed_with_findings).unwrap();
    }
}
