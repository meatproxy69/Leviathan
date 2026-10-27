//! Bounded, exact whole-cell snapshots and replace/clear plans. No decoder,
//! filesystem, event evaluator, clipboard service, routing rewrite or DSP.
//! Document identity belongs to the opaque Editor wrappers; the pure helpers
//! alone do not authorize moving snapshots between documents.
use crate::song::*;
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    io::Write,
};

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "whole-cell-rectangles-v1";
pub const MAX_POSITIONS: usize = 4096;
pub const MAX_CLIPBOARD_BYTES: usize = 1024 * 1024;
pub const MAX_DIFF_BYTES: usize = 2 * 1024 * 1024;
pub const MAX_PAGE: usize = 64;
pub const MAX_VISITS: usize = 65_536;
pub const MAX_PHRASE_CELLS: usize = 32_768;
pub const MAX_VALIDATION_UNITS: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RectScope {
    pub pattern: PatternId,
    pub track: TrackId,
    pub row_start: u32,
    pub row_end: u32,
    pub column_start: u16,
    pub column_end: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PasteDestination {
    pub pattern: PatternId,
    pub track: TrackId,
    pub row_start: u32,
    pub column_start: u16,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellBlockWork {
    pub ordered_track_blocks: usize,
    pub definition_rows: usize,
    pub definition_cells: usize,
    pub ordered_rows: usize,
    pub ordered_cells: usize,
    pub phrase_cells: usize,
    pub instruments: usize,
    /// (definition_cells + ordered_cells + phrase_cells) * max(1,instruments).
    pub validation_units: usize,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellBlockCopySummary {
    pub version: u32,
    pub algorithm: String,
    pub source_scope: RectScope,
    /// Logical read target first, terminal source last; aliases are read only.
    pub source_chain: Vec<TrackAlias>,
    pub rows: u32,
    pub columns: u16,
    pub positions: usize,
    pub stored_cells: usize,
    pub holes: usize,
    pub explicit_default_cells: usize,
    pub serialized_bytes: usize,
    pub warnings: Vec<String>,
}
/// Frozen raw cells/IDs, not a live source selection or source-audio snapshot.
/// No Deserialize: adapters cannot reconstruct authority from client JSON.
#[derive(Debug, Clone, Serialize)]
pub struct CellBlockSnapshot {
    id: String,
    source_document_sha256: String,
    cells: Vec<Option<Cell>>,
    summary: CellBlockCopySummary,
}
impl CellBlockSnapshot {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn source_document_sha256(&self) -> &str {
        &self.source_document_sha256
    }
    pub fn source_scope(&self) -> &RectScope {
        &self.summary.source_scope
    }
    pub fn source_chain(&self) -> &[TrackAlias] {
        &self.summary.source_chain
    }
    pub fn rows(&self) -> u32 {
        self.summary.rows
    }
    pub fn columns(&self) -> u16 {
        self.summary.columns
    }
    /// Row-major positions, including None holes and Some(default Cell).
    pub fn cells(&self) -> &[Option<Cell>] {
        &self.cells
    }
    pub fn summary(&self) -> &CellBlockCopySummary {
        &self.summary
    }
    pub fn page(&self, start: usize, limit: usize) -> Result<CellBlockPage, String> {
        let end = page_end(start, limit, self.cells.len())?;
        Ok(CellBlockPage {
            start,
            total: self.cells.len(),
            cells: (start..end)
                .map(|index| CellBlockEntry {
                    row_offset: (index / usize::from(self.columns())) as u32,
                    column_offset: (index % usize::from(self.columns())) as u16,
                    cell: self.cells[index].clone(),
                })
                .collect(),
        })
    }
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellBlockEntry {
    pub row_offset: u32,
    pub column_offset: u16,
    pub cell: Option<Cell>,
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellBlockPage {
    pub start: usize,
    pub total: usize,
    pub cells: Vec<CellBlockEntry>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum CellBlockRequest {
    PasteReplace {
        clipboard_id: String,
        source: RectScope,
        destination: PasteDestination,
    },
    Clear {
        scope: RectScope,
    },
}
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellBlockChange {
    pub row: u32,
    pub column: u16,
    pub before: Option<Cell>,
    pub after: Option<Cell>,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellBlockContext {
    pub occurrence: usize,
    pub pattern: PatternId,
    pub track: TrackId,
    pub muted: bool,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct CellBlockSummary {
    pub version: u32,
    pub algorithm: String,
    pub destination: RectScope,
    pub positions: usize,
    pub changed_cells: usize,
    pub inserted_cells: usize,
    pub removed_cells: usize,
    pub replaced_cells: usize,
    pub diff_bytes: usize,
    pub affected_aliases: usize,
    pub affected_contexts: usize,
    pub base_work: CellBlockWork,
    pub projected_work: CellBlockWork,
    pub warnings: Vec<String>,
}
#[derive(Debug, Clone)]
pub struct CellBlockPreview {
    id: String,
    base_sha256: String,
    candidate_sha256: String,
    base: Song,
    candidate: Song,
    request: CellBlockRequest,
    summary: CellBlockSummary,
    changes: Vec<CellBlockChange>,
    contexts: Vec<CellBlockContext>,
    aliases: Vec<TrackAlias>,
}
impl CellBlockPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn request(&self) -> &CellBlockRequest {
        &self.request
    }
    pub fn summary(&self) -> &CellBlockSummary {
        &self.summary
    }
    pub fn changes(&self) -> &[CellBlockChange] {
        &self.changes
    }
    pub fn contexts(&self) -> &[CellBlockContext] {
        &self.contexts
    }
    pub fn aliases(&self) -> &[TrackAlias] {
        &self.aliases
    }
    pub fn base_sha256(&self) -> &str {
        &self.base_sha256
    }
    pub fn candidate_sha256(&self) -> &str {
        &self.candidate_sha256
    }
    pub fn is_current(&self, current: &Song) -> Result<bool, String> {
        Ok(current == &self.base && document_sha256(current)? == self.base_sha256)
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if !self.is_current(current)? {
            return Err("Cell-block preview is stale; prepare and review again".into());
        }
        Ok(self.candidate.clone())
    }
}

fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|b| crate::sample_asset::content_sha256(&b))
        .map_err(|e| e.to_string())
}
/// Bounded exact compact Song digest used by asynchronous document wrappers.
/// Historical identity only; no filesystem or source-byte verification.
pub fn document_sha256(song: &Song) -> Result<String, String> {
    crate::project::check_serialized_size(song)?;
    hash(song)
}
fn page_end(start: usize, limit: usize, total: usize) -> Result<usize, String> {
    if !(1..=MAX_PAGE).contains(&limit) || start > total {
        return Err("Cell-block page requires limit 1..64 and start at most total".into());
    }
    Ok(start.saturating_add(limit).min(total))
}
struct Count {
    bytes: usize,
    limit: usize,
    label: &'static str,
}
impl Write for Count {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let next = self
            .bytes
            .checked_add(bytes.len())
            .filter(|&n| n <= self.limit)
            .ok_or_else(|| {
                std::io::Error::other(format!(
                    "{} exceeds {} encoded bytes; choose a smaller rectangle",
                    self.label, self.limit
                ))
            })?;
        self.bytes = next;
        Ok(bytes.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
fn bounded_size(
    value: &impl Serialize,
    limit: usize,
    label: &'static str,
) -> Result<usize, String> {
    let mut count = Count {
        bytes: 0,
        limit,
        label,
    };
    serde_json::to_writer(&mut count, value).map_err(|e| e.to_string())?;
    Ok(count.bytes)
}
fn shape(song: &Song, scope: &RectScope) -> Result<(u32, u16, usize), String> {
    let rows = scope
        .row_end
        .checked_sub(scope.row_start)
        .filter(|&n| n > 0)
        .ok_or("Rectangle rows require nonempty half-open [start,end) bounds")?;
    let columns = scope
        .column_end
        .checked_sub(scope.column_start)
        .filter(|&n| n > 0)
        .ok_or("Rectangle columns require nonempty half-open [start,end) bounds")?;
    let positions = (rows as usize)
        .checked_mul(usize::from(columns))
        .filter(|&n| n <= MAX_POSITIONS)
        .ok_or("Rectangle exceeds 4096 positions, including holes; no truncation")?;
    let pattern = song
        .patterns
        .get(&scope.pattern)
        .ok_or("Unknown rectangle pattern")?;
    let track = song
        .tracks
        .get(&scope.track)
        .ok_or("Unknown rectangle track")?;
    if scope.row_end > pattern.rows || scope.column_end > track.note_columns {
        return Err(
            "Rectangle exceeds existing rows or note columns; no growth, cropping or wrapping"
                .into(),
        );
    }
    Ok((rows, columns, positions))
}

#[derive(Clone, Copy)]
struct Projection {
    pattern: PatternId,
    track: TrackId,
    rows: usize,
    cells: usize,
    create_block: bool,
}
fn track_counts(song: &Song, key: (PatternId, TrackId)) -> (usize, usize) {
    song.patterns[&key.0]
        .tracks
        .get(&key.1)
        .map_or((0, 0), |rows| {
            (
                rows.len(),
                rows.values().fold(0usize, |n, r| n.saturating_add(r.len())),
            )
        })
}
/// Exact structural counts, including projected destination changes through
/// every alias and ordered use. No cell expansion, voice provenance or IO.
fn work(song: &Song, projection: Option<Projection>) -> Result<CellBlockWork, String> {
    let blocks = song
        .order
        .len()
        .checked_mul(song.track_order.len())
        .filter(|&n| n <= MAX_VISITS)
        .ok_or("Cell blocks exceed 65536 ordered track blocks")?;
    let mut alias_count = 0usize;
    let mut terminals = BTreeMap::new();
    let mut patterns = BTreeMap::new();
    let (mut definition_rows, mut definition_cells) = (0usize, 0usize);
    for (&id, pattern) in &song.patterns {
        alias_count = alias_count.saturating_add(pattern.track_aliases.len());
        if alias_count > 4096 {
            return Err("Cell blocks exceed 4096 track aliases".into());
        }
        let (mut rows, mut cells) = (0usize, 0usize);
        for &track in pattern.tracks.keys().chain(pattern.track_aliases.keys()) {
            let terminal = song.resolve_track_source(id, track)?;
            let (add_rows, add_cells) = *terminals.entry(terminal).or_insert_with(|| {
                if let Some(p) = projection.filter(|p| (p.pattern, p.track) == terminal) {
                    (p.rows, p.cells)
                } else {
                    track_counts(song, terminal)
                }
            });
            rows = rows.saturating_add(add_rows);
            cells = cells.saturating_add(add_cells);
        }
        if let Some(p) = projection.filter(|p| p.pattern == id && p.create_block) {
            rows = rows.saturating_add(p.rows);
            cells = cells.saturating_add(p.cells);
        }
        definition_rows = definition_rows.saturating_add(rows);
        definition_cells = definition_cells.saturating_add(cells);
        if definition_rows > MAX_VISITS || definition_cells > MAX_VISITS {
            return Err("Cell blocks exceed 65536 resolved definition rows or cells, including unused aliases/default cells".into());
        }
        patterns.insert(id, (rows, cells));
    }
    let (mut ordered_rows, mut ordered_cells) = (0usize, 0usize);
    for occurrence in &song.order {
        let &(rows, cells) = patterns
            .get(&occurrence.pattern)
            .ok_or("Unknown ordered pattern")?;
        ordered_rows = ordered_rows.saturating_add(rows);
        ordered_cells = ordered_cells.saturating_add(cells);
        if ordered_rows > MAX_VISITS || ordered_cells > MAX_VISITS {
            return Err("Cell blocks exceed 65536 ordered row/cell visits, including empty/default/muted content".into());
        }
    }
    let mut phrase_cells = 0usize;
    for phrase in song.phrases.values() {
        for row in phrase.cells.values() {
            phrase_cells = phrase_cells.saturating_add(row.len());
            if phrase_cells > MAX_PHRASE_CELLS {
                return Err("Cell blocks exceed 32768 phrase-definition cells".into());
            }
        }
    }
    let validation_units = definition_cells
        .checked_add(ordered_cells)
        .and_then(|n| n.checked_add(phrase_cells))
        .and_then(|n| n.checked_mul(song.instruments.len().max(1)))
        .ok_or("Cell-block validation work overflow")?;
    if validation_units > MAX_VALIDATION_UNITS {
        return Err(format!("Cell-block validation work {validation_units} exceeds {MAX_VALIDATION_UNITS} units; reduce content, repeats or instrument definitions"));
    }
    Ok(CellBlockWork {
        ordered_track_blocks: blocks,
        definition_rows,
        definition_cells,
        ordered_rows,
        ordered_cells,
        phrase_cells,
        instruments: song.instruments.len(),
        validation_units,
    })
}
/// Size/work preflight before controller snapshot cloning. This is neither full
/// model validation nor a source assignment, saveability or rendering proof.
pub fn check_document_work(song: &Song) -> Result<CellBlockWork, String> {
    crate::project::check_serialized_size(song)?;
    work(song, None)
}

pub fn copy(song: &Song, scope: &RectScope) -> Result<CellBlockSnapshot, String> {
    let (rows, columns, positions) = shape(song, scope)?;
    check_document_work(song)?;
    song.validate()?;
    let source_chain = song.resolve_track_chain(scope.pattern, scope.track)?;
    let content = song.resolved_track(scope.pattern, scope.track)?;
    let mut cells = Vec::with_capacity(positions);
    for row in scope.row_start..scope.row_end {
        for column in scope.column_start..scope.column_end {
            cells.push(
                content
                    .and_then(|r| r.get(&row))
                    .and_then(|r| r.get(&column))
                    .cloned(),
            );
        }
    }
    let stored_cells = cells.iter().filter(|c| c.is_some()).count();
    let explicit_default_cells = cells
        .iter()
        .filter(|c| c.as_ref() == Some(&Cell::default()))
        .count();
    let source_document_sha256 = hash(song)?;
    let id = hash(&(
        ALGORITHM,
        &source_document_sha256,
        scope,
        &source_chain,
        &cells,
    ))?;
    let mut snapshot=CellBlockSnapshot{id,source_document_sha256,cells,summary:CellBlockCopySummary{
        version:VERSION,algorithm:ALGORITHM.into(),source_scope:scope.clone(),source_chain,rows,columns,positions,stored_cells,
        holes:positions-stored_cells,explicit_default_cells,serialized_bytes:0,
        warnings:vec![
            "Frozen whole cells/IDs, including absent positions and explicit default cells; no source audio or inherited instrument/control memory is copied.".into(),
            "Editor document-owner authority is required. Clipboard survives ordinary edits/Undo/Redo/Save, but not successful New/Open. References resolve against current document mappings at Paste, which may have changed.".into(),
            "Alias sources are read-only resolved snapshots, not live links. Paste replaces holes too; only local destinations on the same logical TrackId are supported.".into(),
        ],
    }};
    // The reported size is part of the serialized snapshot. Stabilize its small
    // decimal width, then enforce the exact full snapshot limit, not just cells.
    loop {
        let size = bounded_size(&snapshot, MAX_CLIPBOARD_BYTES, "Cell clipboard")?;
        if size == snapshot.summary.serialized_bytes {
            break;
        }
        snapshot.summary.serialized_bytes = size;
    }
    Ok(snapshot)
}

pub fn paste_preview(
    song: &Song,
    clipboard: &CellBlockSnapshot,
    destination: &PasteDestination,
) -> Result<CellBlockPreview, String> {
    if clipboard.source_scope().track != destination.track {
        return Err(
            "Paste requires the same logical TrackId as Copy; no cross-track routing".into(),
        );
    }
    let scope = RectScope {
        pattern: destination.pattern,
        track: destination.track,
        row_start: destination.row_start,
        row_end: destination
            .row_start
            .checked_add(clipboard.rows())
            .ok_or("Paste row bound overflow")?,
        column_start: destination.column_start,
        column_end: destination
            .column_start
            .checked_add(clipboard.columns())
            .ok_or("Paste column bound overflow")?,
    };
    edit_preview(
        song,
        &scope,
        Some(clipboard),
        CellBlockRequest::PasteReplace {
            clipboard_id: clipboard.id.clone(),
            source: clipboard.source_scope().clone(),
            destination: destination.clone(),
        },
    )
}
pub fn clear_preview(song: &Song, scope: &RectScope) -> Result<CellBlockPreview, String> {
    edit_preview(
        song,
        scope,
        None,
        CellBlockRequest::Clear {
            scope: scope.clone(),
        },
    )
}
fn cell_equal(a: &Option<Cell>, b: &Option<Cell>) -> Result<bool, String> {
    if a != b {
        return Ok(false);
    }
    Ok(serde_json::to_vec(a).map_err(|e| e.to_string())?
        == serde_json::to_vec(b).map_err(|e| e.to_string())?)
}
fn edit_preview(
    song: &Song,
    scope: &RectScope,
    clipboard: Option<&CellBlockSnapshot>,
    request: CellBlockRequest,
) -> Result<CellBlockPreview, String> {
    let (_, columns, positions) = shape(song, scope)?;
    song.require_local_track(scope.pattern, scope.track)?;
    let base_work = check_document_work(song)?;
    let content = song.patterns[&scope.pattern].tracks.get(&scope.track);
    let mut changes = Vec::new();
    let mut new_rows = BTreeSet::new();
    let (mut inserted, mut removed, mut replaced) = (0usize, 0usize, 0usize);
    for index in 0..positions {
        let row = scope.row_start + (index / usize::from(columns)) as u32;
        let column = scope.column_start + (index % usize::from(columns)) as u16;
        let before = content
            .and_then(|r| r.get(&row))
            .and_then(|r| r.get(&column))
            .cloned();
        let after = clipboard.and_then(|c| c.cells[index].clone());
        if cell_equal(&before, &after)? {
            continue;
        }
        match (before.is_some(), after.is_some()) {
            (false, true) => inserted += 1,
            (true, false) => removed += 1,
            (true, true) => replaced += 1,
            _ => unreachable!(),
        }
        if after.is_some() && content.is_none_or(|r| !r.contains_key(&row)) {
            new_rows.insert(row);
        }
        changes.push(CellBlockChange {
            row,
            column,
            before,
            after,
        });
    }
    let (old_rows, old_cells) = track_counts(song, (scope.pattern, scope.track));
    let projection = Projection {
        pattern: scope.pattern,
        track: scope.track,
        rows: old_rows + new_rows.len(),
        cells: old_cells - removed + inserted,
        create_block: content.is_none() && inserted > 0,
    };
    let projected_work = work(song, Some(projection))?;
    let diff_bytes = bounded_size(&changes, MAX_DIFF_BYTES, "Cell-block diff")?;
    // Only now run potentially incoming-reference-expanding model validation.
    song.validate()?;
    for change in &changes {
        if let Some(cell) = &change.after {
            song.validate_cell(cell)?;
        }
    }
    let mut candidate = song.clone();
    for change in &changes {
        let tracks = &mut candidate.patterns.get_mut(&scope.pattern).unwrap().tracks;
        if let Some(cell) = &change.after {
            tracks
                .entry(scope.track)
                .or_default()
                .entry(change.row)
                .or_default()
                .insert(change.column, cell.clone());
        } else if let Some(row) = tracks
            .get_mut(&scope.track)
            .and_then(|r| r.get_mut(&change.row))
        {
            row.remove(&change.column); // Deliberately preserve empty containers.
        }
    }
    crate::project::check_serialized_size(&candidate)?;
    candidate.validate()?;
    let (aliases, contexts) = if changes.is_empty() {
        (Vec::new(), Vec::new())
    } else {
        affected_contexts(song, scope)?
    };
    let summary=CellBlockSummary{version:VERSION,algorithm:ALGORITHM.into(),destination:scope.clone(),positions,
        changed_cells:changes.len(),inserted_cells:inserted,removed_cells:removed,replaced_cells:replaced,diff_bytes,
        affected_aliases:aliases.len(),affected_contexts:contexts.len(),base_work,projected_work,
        warnings:vec![
            "Exact whole-cell replacement/clear, including OFF, scalars, effects, phrase triggers and signed zero. Holes remove destination cells; explicit default cells remain stored. Existing empty containers are preserved.".into(),
            "Only one local pattern/track rectangle changes. Repeated occurrences and aliases reading that content see the edit too; no implicit Make unique, growth, cropping, merge, source assignment or clock change.".into(),
            "Copied IDs resolve against current document definitions, not frozen source/mapping semantics. Incoming instrument/volume/pan memory, phrase replacement, RR order, choke and release behavior can differ after editing; no retained-audio or event-count equivalence is promised.".into(),
            "Model validation is not a full rendering proof. No filesystem, source hash, decoding, sample assignment or audio scheduling occurs; normal playback/export budgets may still reject the edited Song.".into(),
        ]};
    let base_sha256 = hash(song)?;
    let candidate_sha256 = hash(&candidate)?;
    let id = hash(&(
        ALGORITHM,
        &base_sha256,
        &request,
        &candidate_sha256,
        &changes,
    ))?;
    Ok(CellBlockPreview {
        id,
        base_sha256,
        candidate_sha256,
        base: song.clone(),
        candidate,
        request,
        summary,
        changes,
        contexts,
        aliases,
    })
}
fn affected_contexts(
    song: &Song,
    scope: &RectScope,
) -> Result<(Vec<TrackAlias>, Vec<CellBlockContext>), String> {
    let mut blocks = BTreeSet::from([(scope.pattern, scope.track)]);
    let mut aliases = Vec::new();
    for (&pattern, p) in &song.patterns {
        for &track in p.track_aliases.keys() {
            if song.resolve_track_source(pattern, track)? == (scope.pattern, scope.track) {
                blocks.insert((pattern, track));
                aliases.push(TrackAlias { pattern, track });
            }
        }
    }
    let mut contexts = Vec::new();
    for (occurrence, o) in song.order.iter().enumerate() {
        for &track in &song.track_order {
            if blocks.contains(&(o.pattern, track)) {
                contexts.push(CellBlockContext {
                    occurrence,
                    pattern: o.pattern,
                    track,
                    muted: o.muted_tracks.contains(&track),
                });
            }
        }
    }
    Ok((aliases, contexts))
}
