//! Editor state is separate from persistent song content and future playback state.
use crate::song::*;
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Field {
    Note,
    Instrument,
    Velocity,
    Pan,
    Delay,
}
impl Field {
    pub const ALL: [Self; 5] = [
        Self::Note,
        Self::Instrument,
        Self::Velocity,
        Self::Pan,
        Self::Delay,
    ];
    pub fn index(self) -> usize {
        Self::ALL.iter().position(|f| *f == self).unwrap()
    }
}
const HISTORY_LIMIT: usize = 64;
#[derive(Debug, Clone)]
struct Snapshot {
    song: Song,
    occurrence: usize,
    track: TrackId,
    column: u16,
    row: u32,
    field: Field,
}
#[derive(Debug, Clone)]
pub struct History {
    undo: Vec<Snapshot>,
    redo: Vec<Snapshot>,
    saved: Song,
    revision: u64,
    document_owner: Arc<()>,
    last_change_mix_only: bool,
}
impl Default for History {
    fn default() -> Self {
        Self {
            undo: Vec::new(),
            redo: Vec::new(),
            saved: example_song(),
            revision: 0,
            document_owner: Arc::new(()),
            last_change_mix_only: false,
        }
    }
}
#[derive(Debug, Clone)]
pub struct Editor {
    pub song: Song,
    pub occurrence: usize,
    pub track: TrackId,
    pub column: u16,
    pub row: u32,
    pub field: Field,
    pub edit_mode: bool,
    pub edit_step: u32,
    pub octave: u8,
    pub dirty: bool,
    pub history: History,
}
impl Default for Editor {
    fn default() -> Self {
        Self {
            song: example_song(),
            occurrence: 0,
            track: TrackId(1),
            column: 0,
            row: 0,
            field: Field::Note,
            edit_mode: false,
            edit_step: 1,
            octave: 4,
            dirty: false,
            history: History::default(),
        }
    }
}
/// Cheap document identity, not a serialized nonce or a playback capability.
/// Successful New/Open replaces it; ordinary edits, Save and Undo/Redo do not.
#[derive(Debug, Clone)]
pub struct DocumentOwner {
    identity: Arc<()>,
}
impl DocumentOwner {
    pub(crate) fn same_owner(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.identity, &other.identity)
    }
}
/// One bounded worker input, not an Editor/history clone. Authority cannot be
/// reconstructed from serialized data or attached to arbitrary core snapshots.
#[derive(Debug)]
pub struct DocumentCapture {
    owner: Arc<()>,
    revision: u64,
    song: Song,
}
#[derive(Debug, Clone)]
pub struct DocumentClipboard {
    owner: Arc<()>,
    revision: u64,
    snapshot: crate::cell_blocks::CellBlockSnapshot,
}
#[derive(Debug, Clone)]
pub struct DocumentCellPreview {
    owner: Arc<()>,
    revision: u64,
    preview: crate::cell_blocks::CellBlockPreview,
}
#[derive(Debug, Clone)]
pub struct DocumentTrackMixPreview {
    owner: Arc<()>,
    revision: u64,
    preview: crate::track_mix::TrackMixPreview,
}
impl DocumentCapture {
    pub fn revision(&self) -> u64 {
        self.revision
    }
    pub fn song(&self) -> &Song {
        &self.song
    }
    pub fn preview_track_mix(
        &self,
        config: crate::track_mix::TrackMixConfig,
    ) -> Result<DocumentTrackMixPreview, String> {
        Ok(DocumentTrackMixPreview {
            owner: Arc::clone(&self.owner),
            revision: self.revision,
            preview: crate::track_mix::preview(&self.song, config)?,
        })
    }
    pub fn copy_cells(
        &self,
        scope: &crate::cell_blocks::RectScope,
    ) -> Result<DocumentClipboard, String> {
        Ok(DocumentClipboard {
            owner: Arc::clone(&self.owner),
            revision: self.revision,
            snapshot: crate::cell_blocks::copy(&self.song, scope)?,
        })
    }
    pub fn paste_cells_preview(
        &self,
        clipboard: &DocumentClipboard,
        destination: &crate::cell_blocks::PasteDestination,
    ) -> Result<DocumentCellPreview, String> {
        if !Arc::ptr_eq(&self.owner, &clipboard.owner) {
            return Err("Clipboard belongs to another document; Copy again after New/Open".into());
        }
        Ok(DocumentCellPreview {
            owner: Arc::clone(&self.owner),
            revision: self.revision,
            preview: crate::cell_blocks::paste_preview(
                &self.song,
                &clipboard.snapshot,
                destination,
            )?,
        })
    }
    pub fn clear_cells_preview(
        &self,
        scope: &crate::cell_blocks::RectScope,
    ) -> Result<DocumentCellPreview, String> {
        Ok(DocumentCellPreview {
            owner: Arc::clone(&self.owner),
            revision: self.revision,
            preview: crate::cell_blocks::clear_preview(&self.song, scope)?,
        })
    }
}
impl DocumentClipboard {
    pub fn snapshot(&self) -> &crate::cell_blocks::CellBlockSnapshot {
        &self.snapshot
    }
    /// The revision at Copy, not a requirement for later same-document Paste.
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
impl DocumentCellPreview {
    pub fn preview(&self) -> &crate::cell_blocks::CellBlockPreview {
        &self.preview
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
impl DocumentTrackMixPreview {
    pub fn preview(&self) -> &crate::track_mix::TrackMixPreview {
        &self.preview
    }
    pub fn revision(&self) -> u64 {
        self.revision
    }
}
/// A delivery hint only: failure must not broaden ordinary edit/restore refusal.
/// It is evaluated once per changed history transition, never during UI paint.
fn track_mix_only_change(before: &Song, after: &Song) -> bool {
    if before.track_mix.len() > crate::track_mix::MAX_TRACKS
        || after.track_mix.len() > crate::track_mix::MAX_TRACKS
    {
        return false;
    }
    let (Ok(left), Ok(right)) = (
        serde_json::to_vec(&before.track_mix),
        serde_json::to_vec(&after.track_mix),
    ) else {
        return false;
    };
    left != right && crate::track_mix::same_playing_content(before, after).unwrap_or(false)
}
impl Editor {
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            song: self.song.clone(),
            occurrence: self.occurrence,
            track: self.track,
            column: self.column,
            row: self.row,
            field: self.field,
        }
    }
    fn restore(&mut self, state: Snapshot) {
        let mix_only = track_mix_only_change(&self.song, &state.song);
        self.song = state.song;
        self.occurrence = state.occurrence;
        self.track = state.track;
        self.column = state.column;
        self.row = state.row;
        self.field = state.field;
        // Undo/Redo remain infallible, including legacy oversized snapshots.
        // If exact equality cannot be established, never falsely claim clean.
        self.dirty =
            !crate::project::same_serialized_song(&self.song, &self.history.saved).unwrap_or(false);
        self.history.revision = self.history.revision.wrapping_add(1);
        self.history.last_change_mix_only = mix_only;
    }
    fn commit(&mut self, candidate: Song) -> Result<(), String> {
        candidate.validate()?;
        for (id, source) in &self.song.samples {
            if candidate.samples.get(id).is_some_and(|next| {
                next != source || next.path.as_os_str() != source.path.as_os_str()
            }) {
                return Err(format!(
                    "Source {} is immutable; import changed bytes, metadata, or paths with a new source ID",
                    id.0
                ));
            }
        }
        for (id, groove) in &self.song.grooves {
            if candidate.grooves.get(id).is_some_and(|next| next != groove) {
                return Err(format!(
                    "Groove {} is immutable; capture changed settings with a new groove ID",
                    id.0
                ));
            }
        }
        let unchanged = crate::project::same_serialized_song(&candidate, &self.song)?;
        let dirty = !crate::project::same_serialized_song(&candidate, &self.history.saved)?;
        if unchanged {
            return Ok(());
        }
        let mix_only = track_mix_only_change(&self.song, &candidate);
        self.history.undo.push(self.snapshot());
        if self.history.undo.len() > HISTORY_LIMIT {
            self.history.undo.remove(0);
        }
        self.history.redo.clear();
        self.song = candidate;
        self.dirty = dirty;
        self.history.revision = self.history.revision.wrapping_add(1);
        self.history.last_change_mix_only = mix_only;
        Ok(())
    }
    /// Content generation for stale-preview guards. Navigation, saving and no-op
    /// edits preserve it; successful edits, Undo/Redo and document replacement do not.
    pub fn revision(&self) -> u64 {
        self.history.revision
    }
    pub fn document_owner(&self) -> DocumentOwner {
        DocumentOwner {
            identity: Arc::clone(&self.history.document_owner),
        }
    }
    pub fn owns_document(&self, owner: &DocumentOwner) -> bool {
        self.document_owner().same_owner(owner)
    }
    /// Observe only on an actual revision transition. Save and true no-ops retain
    /// the revision; an explicit no-op Apply may separately request a live retry.
    /// This is not authorization or proof that a runtime update was delivered.
    pub fn track_mix_change_revision(&self) -> Option<u64> {
        self.history
            .last_change_mix_only
            .then_some(self.history.revision)
    }
    /// Edit the actual Song clock, not a demo/export-only tempo control.
    pub fn set_timing(&mut self, bpm: f64, lines_per_beat: u16) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.bpm = bpm;
        candidate.lines_per_beat = lines_per_beat;
        self.commit(candidate)
    }
    /// Apply a complete, validated content batch as one undoable edit.
    /// This is not a load: Edit Mode, history, and the saved-state baseline remain intact.
    /// Keep the selected position/track when possible and clamp after geometry changes.
    /// A no-op preserves the cursor and both history stacks exactly.
    pub fn apply_document_edit(&mut self, candidate: Song) -> Result<(), String> {
        self.protect()?;
        let before_revision = self.revision();
        self.commit(candidate)?;
        if self.revision() == before_revision {
            return Ok(());
        }
        if self.song.order.is_empty() || self.song.track_order.is_empty() {
            self.occurrence = 0;
            self.track = TrackId(0);
            self.column = 0;
            self.row = 0;
            return Ok(());
        }
        self.occurrence = self.occurrence.min(self.song.order.len() - 1);
        if !self.song.tracks.contains_key(&self.track) {
            self.track = self.song.track_order[0];
        }
        self.column = self
            .column
            .min(self.song.tracks[&self.track].note_columns - 1);
        self.row = self.row.min(self.pattern().rows - 1);
        Ok(())
    }
    /// Commit a worker/compiler preview only against its original document.
    /// Capture revision and Song together before preparing the preview. The
    /// content comparison also catches unchecked draft changes outside this API.
    pub fn apply_document_edit_if_current(
        &mut self,
        expected_revision: u64,
        expected_song: &Song,
        candidate: Song,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision
            || !crate::project::same_serialized_song(&self.song, expected_song)?
        {
            return Err(
                "Preview is stale; the document changed. Prepare a new preview before applying"
                    .into(),
            );
        }
        self.apply_document_edit(candidate)
    }
    /// Capture only one size/work-bounded Song for pure worker preparation.
    /// Copy/inspection are allowed in Read Mode and never change history.
    pub fn capture_cell_document(&self) -> Result<DocumentCapture, String> {
        crate::cell_blocks::check_document_work(&self.song)?;
        Ok(DocumentCapture {
            owner: Arc::clone(&self.history.document_owner),
            revision: self.revision(),
            song: self.song.clone(),
        })
    }
    /// Pure, bounded one-Song input for a worker; Read Mode may prepare/review.
    pub fn capture_track_mix_document(&self) -> Result<DocumentCapture, String> {
        crate::track_mix::check_document_work(&self.song)?;
        Ok(DocumentCapture {
            owner: Arc::clone(&self.history.document_owner),
            revision: self.revision(),
            song: self.song.clone(),
        })
    }
    pub fn validate_track_mix_preview(
        &self,
        preview: &DocumentTrackMixPreview,
    ) -> Result<(), String> {
        if !Arc::ptr_eq(&self.history.document_owner, &preview.owner)
            || self.revision() != preview.revision
            || !preview.preview.is_current(&self.song)?
        {
            return Err("Track mix preview is stale; prepare and review again".into());
        }
        Ok(())
    }
    /// Commit persisted settings only. Runtime submission is a separate caller
    /// action: a stopped/stale session must never roll back this valid document.
    pub fn commit_track_mix_preview(
        &mut self,
        preview: &DocumentTrackMixPreview,
    ) -> Result<(), String> {
        self.protect()?;
        self.validate_track_mix_preview(preview)?;
        self.commit(preview.preview.commit(&self.song)?)
    }
    pub fn cell_clipboard_is_current_document(&self, clipboard: &DocumentClipboard) -> bool {
        Arc::ptr_eq(&self.history.document_owner, &clipboard.owner)
    }
    pub fn cell_preview_is_current_document(&self, preview: &DocumentCellPreview) -> bool {
        Arc::ptr_eq(&self.history.document_owner, &preview.owner)
    }
    /// Validate asynchronous Copy completion before publishing a new clipboard.
    /// A clipboard already accepted by a controller survives later content edits.
    pub fn validate_cell_copy(&self, clipboard: &DocumentClipboard) -> Result<(), String> {
        if !self.cell_clipboard_is_current_document(clipboard)
            || self.revision() != clipboard.revision
            || crate::cell_blocks::document_sha256(&self.song)?
                != clipboard.snapshot.source_document_sha256()
        {
            return Err("Cell Copy is stale; copy the current document again".into());
        }
        Ok(())
    }
    /// Read-only READY/inspection guard; Edit Mode is required only for commit.
    pub fn validate_cell_preview(&self, preview: &DocumentCellPreview) -> Result<(), String> {
        if !self.cell_preview_is_current_document(preview)
            || self.revision() != preview.revision
            || !preview.preview.is_current(&self.song)?
        {
            return Err("Cell-block preview is stale; prepare and review again".into());
        }
        Ok(())
    }
    pub fn commit_cell_block_preview(
        &mut self,
        preview: &DocumentCellPreview,
    ) -> Result<(), String> {
        self.protect()?;
        self.validate_cell_preview(preview)?;
        let candidate = preview.preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// A nonempty legacy-path assignment replaces a region mapping. Clearing the
    /// legacy path alone preserves any existing region mapping.
    pub fn assign_sample(
        &mut self,
        instrument: InstrumentId,
        sample: Option<std::path::PathBuf>,
    ) -> Result<(), String> {
        self.protect()?;
        let sample = sample
            .map(|path| {
                path.canonicalize()
                    .map(crate::project::normalize_canonical_path)
                    .map_err(|e| format!("Sample path: {e}"))
            })
            .transpose()?;
        if sample.as_ref().is_some_and(|p| !p.is_file()) {
            return Err("Sample must be a file".into());
        }
        let mut candidate = self.song.clone();
        let target = candidate
            .instruments
            .get_mut(&instrument)
            .ok_or("Unknown instrument")?;
        if sample.is_some() {
            target.region = None;
            target.kind = InstrumentKind::Pitched;
        }
        target.sample = sample;
        self.commit(candidate)
    }
    /// Register imported immutable metadata; callers authorize and decode the source first.
    pub fn register_sample(&mut self, id: SampleId, sample: SampleAsset) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.register_sample(id, sample)?;
        self.commit(candidate)
    }
    /// Referenced sources cannot be deleted, including references from unmapped regions.
    pub fn remove_sample(&mut self, id: SampleId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_sample(id)?;
        self.commit(candidate)
    }
    pub fn upsert_region(&mut self, id: RegionId, region: SampleRegion) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.upsert_region(id, region)?;
        self.commit(candidate)
    }
    /// Replace the shared region's playback options without changing its source,
    /// bounds, root or users. None explicitly disables the respective option.
    /// This is a model-only edit; source-dependent FX are checked at evaluation.
    pub fn set_region_playback(
        &mut self,
        id: RegionId,
        amp_envelope: Option<AmpEnvelope>,
        forward_loop: Option<ForwardLoop>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_region_playback(id, amp_envelope, forward_loop)?;
        self.commit(candidate)
    }
    /// Store explicitly reviewed native descriptors/labels against their exact base.
    /// Callers reauthorize and verify measured source bytes before this pure commit.
    /// Metadata is not playback authority; no region, note, clock or cursor moves.
    pub fn commit_region_catalog_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::slice_features::CatalogPreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err("Slice catalog review is stale; analyze and review again".into());
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Set or clear an explicit model-bound user role; this does not analyze audio
    /// or claim source-byte verification. Native descriptors remain independent.
    pub fn set_region_role(
        &mut self,
        id: RegionId,
        label: Option<crate::slice_features::RoleLabel>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_region_role(id, label)?;
        self.commit(candidate)
    }
    /// Clear measured features without silently clearing a separately reviewed role.
    pub fn clear_region_features(&mut self, id: RegionId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.clear_region_features(id)?;
        self.commit(candidate)
    }
    /// Create one empty track as a single undoable document edit. The returned
    /// stable ID is suitable for selecting it in a native UI.
    pub fn add_track(&mut self, name: String) -> Result<TrackId, String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let id = candidate.add_track(name)?;
        self.commit(candidate)?;
        Ok(id)
    }
    /// Create one unassigned pitched instrument as a single undoable document
    /// edit. Native controls can use this operation without changing existing
    /// instrument assignments.
    pub fn add_instrument(&mut self, name: String) -> Result<InstrumentId, String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let id = candidate.add_instrument(name)?;
        self.commit(candidate)?;
        Ok(id)
    }
    pub fn remove_region(&mut self, id: RegionId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_region(id)?;
        self.commit(candidate)
    }
    /// A region mapping replaces a legacy path; clearing only removes the region mapping.
    pub fn assign_region(
        &mut self,
        instrument: InstrumentId,
        region: Option<RegionId>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.assign_region(instrument, region)?;
        self.commit(candidate)
    }
    /// Add an empty melodic map and fresh instrument without rebinding existing
    /// content. Coverage is checked when notes are evaluated, not invented here.
    pub fn create_multisample_instrument(
        &mut self,
        name: String,
    ) -> Result<(InstrumentId, MultisampleId), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let ids = candidate.create_multisample_instrument(name)?;
        self.commit(candidate)?;
        Ok(ids)
    }
    /// Replace a shared map in full; every instrument using its stable ID sees
    /// this edit. Copy to a fresh ID explicitly for independent map metadata.
    pub fn upsert_multisample(
        &mut self,
        id: MultisampleId,
        map: MultisampleInstrument,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.upsert_multisample(id, map)?;
        self.commit(candidate)
    }
    /// Replace all rectangles together so adjacent boundaries can move without
    /// exposing an invalid intermediate overlap. Missing coverage is permitted.
    pub fn set_keyzones(
        &mut self,
        id: MultisampleId,
        zones: std::collections::BTreeMap<KeyZoneId, KeyZone>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_keyzones(id, zones)?;
        self.commit(candidate)
    }
    pub fn remove_multisample(&mut self, id: MultisampleId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_multisample(id)?;
        self.commit(candidate)
    }
    /// Some explicitly replaces the source kind and clears legacy assignments.
    /// None detaches only Multisample; Pitched/DrumKit stay unchanged. This differs
    /// deliberately from the older assign_kit(None) reset-to-Pitched operation.
    pub fn assign_multisample(
        &mut self,
        instrument: InstrumentId,
        map: Option<MultisampleId>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.assign_multisample(instrument, map)?;
        self.commit(candidate)
    }
    /// Add an empty kit and fresh instrument as one edit, leaving the cursor and
    /// every existing instrument assignment intact.
    pub fn create_drum_kit_instrument(
        &mut self,
        name: String,
    ) -> Result<(InstrumentId, DrumKitId), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        let ids = candidate.create_drum_kit_instrument(name)?;
        self.commit(candidate)?;
        Ok(ids)
    }
    pub fn upsert_drum_kit(&mut self, id: DrumKitId, kit: DrumKitInstrument) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.upsert_drum_kit(id, kit)?;
        self.commit(candidate)
    }
    pub fn remove_drum_kit(&mut self, id: DrumKitId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_drum_kit(id)?;
        self.commit(candidate)
    }
    pub fn upsert_drum_pad(
        &mut self,
        kit: DrumKitId,
        id: PadId,
        pad: DrumPad,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.upsert_drum_pad(kit, id, pad)?;
        self.commit(candidate)
    }
    pub fn remove_drum_pad(&mut self, kit: DrumKitId, id: PadId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_drum_pad(kit, id)?;
        self.commit(candidate)
    }
    /// Explicitly replace the pad's single-region or velocity-zone sources.
    /// None plus empty zones unmaps; other pad parameters and cells are retained.
    pub fn set_pad_sources(
        &mut self,
        kit: DrumKitId,
        pad: PadId,
        region: Option<RegionId>,
        velocity_zones: Vec<VelocityZone>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_pad_sources(kit, pad, region, velocity_zones)?;
        self.commit(candidate)
    }
    /// Explicit kit mapping replaces legacy/region assignment; None returns to Pitched.
    pub fn assign_kit(
        &mut self,
        instrument: InstrumentId,
        kit: Option<DrumKitId>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.assign_kit(instrument, kit)?;
        self.commit(candidate)
    }
    /// Insert an ordinary tracker note from an explicitly selected mapped kit pad.
    /// Retain compatible scalar/effect fields; never silently replace a phrase trigger.
    /// Like keyboard note entry, a successful insertion advances by edit_step.
    pub fn insert_pad_note(&mut self, instrument: InstrumentId, pad: PadId) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let mut cell = self.cell();
        if cell.phrase.is_some() {
            return Err("Clear the selected phrase trigger before inserting a pad note".into());
        }
        let kind = &self
            .song
            .instruments
            .get(&instrument)
            .ok_or("Unknown instrument")?
            .kind;
        let InstrumentKind::DrumKit { kit } = kind else {
            return Err("Selected instrument is not assigned to a drum kit".into());
        };
        let pad = self
            .song
            .drum_kits
            .get(kit)
            .ok_or("Unknown drum kit")?
            .pads
            .get(&pad)
            .ok_or("Unknown drum pad")?;
        if !pad.has_source() {
            return Err("Assign source regions to the pad before inserting its note".into());
        }
        cell.note = Note::Note(pad.note);
        cell.instrument = Some(instrument);
        self.set_cell(cell)?;
        self.move_rows(self.edit_step as i64);
        Ok(())
    }
    /// Apply one guarded edit to ordinary cells through the drum-step projection.
    /// Navigation and the tracker cursor are deliberately unaffected.
    pub fn edit_drum_step(
        &mut self,
        expected_revision: u64,
        projection: &crate::drum_steps::StepProjection,
        edit: &crate::drum_steps::StepEdit,
    ) -> Result<(), String> {
        self.protect()?;
        if expected_revision != self.revision() {
            return Err("Drum step projection is stale; inspect again".into());
        }
        let candidate = crate::drum_steps::apply(&self.song, projection, edit)?;
        self.commit(candidate)
    }
    /// Persist accepted manual boundaries, independently from replaceable detector proposals.
    pub fn upsert_marker_set(&mut self, id: MarkerSetId, set: MarkerSet) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.upsert_marker_set(id, set)?;
        self.commit(candidate)
    }
    pub fn remove_marker_set(&mut self, id: MarkerSetId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_marker_set(id)?;
        self.commit(candidate)
    }
    pub fn set_marker_lock(
        &mut self,
        id: MarkerSetId,
        frame: u64,
        locked: bool,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_marker_lock(id, frame, locked)?;
        self.commit(candidate)
    }
    pub fn add_marker(&mut self, id: MarkerSetId, frame: u64) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.add_marker(id, frame)?;
        self.commit(candidate)
    }
    pub fn move_marker(&mut self, id: MarkerSetId, from: u64, to: u64) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.move_marker(id, from, to)?;
        self.commit(candidate)
    }
    pub fn remove_marker(&mut self, id: MarkerSetId, frame: u64) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_marker(id, frame)?;
        self.commit(candidate)
    }
    /// Commit an opaque compiler preview as one history entry. This performs no
    /// IO: callers must authorize and re-verify the preview source on a worker
    /// before applying, while retaining the originally captured revision.
    pub fn commit_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::break_compiler::CompilePreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err(
                "Preview is stale; the document changed. Prepare a new preview before applying"
                    .into(),
            );
        }
        let candidate = preview.commit(&self.song)?;
        self.apply_document_edit(candidate)
    }
    /// Register a reviewed marker bank as one protected metadata transaction.
    /// The opaque plan guards the exact base, including serialized float bits.
    /// No source IO, selection, navigation, note insertion or audition occurs here;
    /// adapters must freshly verify the selected source before calling this.
    pub fn commit_marker_bank_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::marker_bank::MarkerBankPreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err(
                "Marker bank preview is stale; prepare a new preview before applying".into(),
            );
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Commit a reviewed, purely musical transform against its captured document.
    /// Scope and field masks belong to the opaque plan, never the current cursor.
    /// No IO or navigation occurs; a true no-op preserves history and saved state.
    pub fn commit_transform_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::transforms::TransformPreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err("Transform preview is stale; prepare a new preview before applying".into());
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Commit reviewed seeded whole-cell thinning against its exact source document.
    /// The opaque plan already proves retained note starts against the shared model
    /// evaluator. No gate, audio IO or navigation runs here; a no-op keeps redo.
    pub fn commit_thin_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::mutations::ThinPreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err("THIN preview is stale; prepare a new preview before applying".into());
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Register the exact reviewed marker capture without re-extracting or IO.
    /// Template identity and its captured authority are immutable after commit.
    pub fn commit_groove_capture_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::groove::CapturePreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err("Groove capture is stale; prepare a new preview before applying".into());
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Bake reviewed groove timing into ordinary cells in one history entry.
    /// The opaque scope is authoritative; cursor, markers and templates stay put.
    pub fn commit_groove_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::groove::GroovePreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err("Groove preview is stale; prepare a new preview before applying".into());
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Applied cells contain baked timing, not a live groove reference. Historical
    /// capture provenance likewise does not hold source/marker deletion references.
    pub fn remove_groove(&mut self, id: GrooveId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_groove(id)?;
        self.commit(candidate)
    }
    /// Append reviewed generated ordinary content as one undoable operation.
    /// The opaque plan owns its seed/configuration and original document; no
    /// generation or IO runs during commit. Preserve a valid selection, or
    /// initialize/clamp the empty-document cursor after the first append.
    pub fn commit_generation_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::generation::GenerationPreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err(
                "Generation preview is stale; prepare a new preview before applying".into(),
            );
        }
        let candidate = preview.commit(&self.song)?;
        self.apply_document_edit(candidate)
    }
    /// Register a successfully published render as one metadata-only transaction.
    /// Callers reauthorize and verify the artifact off-thread before this no-I/O
    /// commit. Stale/failed commits, Undo and Redo never delete or recreate files.
    pub fn commit_resample_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::resample::ResamplePreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err(
                "Rendered asset registration is stale; the published file is retained".into(),
            );
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Apply only the explicitly selected tempo candidate to its reviewed marker set.
    /// No analysis, source IO, phase application or navigation occurs here. Source
    /// and cache identity describe historical evidence, not a fresh byte check.
    /// The opaque plan owns the exact base/target; a true no-op preserves redo.
    pub fn commit_tempo_preview(
        &mut self,
        expected_revision: u64,
        preview: &crate::tempo::UseBpmPreview,
    ) -> Result<(), String> {
        self.protect()?;
        if self.revision() != expected_revision {
            return Err(
                "Tempo candidate review is stale; prepare a new analysis before applying".into(),
            );
        }
        let candidate = preview.commit(&self.song)?;
        self.commit(candidate)
    }
    /// Create or replace a stable phrase definition without changing the song cursor.
    pub fn upsert_phrase(&mut self, id: PhraseId, phrase: Phrase) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.upsert_phrase(id, phrase)?;
        self.commit(candidate)
    }
    /// Refuse deletion while any persistent song cell still refers to this phrase.
    pub fn remove_phrase(&mut self, id: PhraseId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.remove_phrase(id)?;
        self.commit(candidate)
    }
    pub fn set_phrase_cell(
        &mut self,
        id: PhraseId,
        row: u32,
        column: u16,
        cell: Cell,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_phrase_cell(id, row, column, cell)?;
        self.commit(candidate)
    }
    /// Replace only the selected song cell's phrase reference; None clears that reference.
    pub fn set_phrase_trigger(&mut self, trigger: Option<PhraseTrigger>) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let mut cell = self.cell();
        cell.phrase = trigger;
        self.set_cell(cell)
    }
    pub fn can_undo(&self) -> bool {
        !self.history.undo.is_empty()
    }
    pub fn can_redo(&self) -> bool {
        !self.history.redo.is_empty()
    }
    pub fn undo(&mut self) {
        if let Some(state) = self.history.undo.pop() {
            self.history.redo.push(self.snapshot());
            self.restore(state);
        }
    }
    pub fn redo(&mut self) {
        if let Some(state) = self.history.redo.pop() {
            self.history.undo.push(self.snapshot());
            self.restore(state);
        }
    }
    pub fn mark_saved(&mut self) {
        self.history.saved = self.song.clone();
        self.dirty = false;
    }
    /// Called only after a successful validated load. Empty documents are supported
    /// by the UI's empty-state view, with no unchecked pattern access.
    pub fn replace_document(&mut self, song: Song) -> Result<(), String> {
        song.validate()?;
        self.history.document_owner = Arc::new(());
        self.history.last_change_mix_only = false;
        self.song = song;
        self.history.revision = self.history.revision.wrapping_add(1);
        self.occurrence = 0;
        self.row = 0;
        self.column = 0;
        self.field = Field::Note;
        self.track = self.song.track_order.first().copied().unwrap_or(TrackId(0));
        self.edit_mode = false;
        self.history.undo.clear();
        self.history.redo.clear();
        self.mark_saved();
        Ok(())
    }
    pub fn has_grid(&self) -> bool {
        self.checked_cursor().is_ok()
    }
    /// Link an empty block, or explicitly unlink to an empty local block.
    /// Neither operation implicitly edits or deletes the shared source content.
    pub fn set_track_alias(
        &mut self,
        pattern: PatternId,
        track: TrackId,
        source: Option<TrackAlias>,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_track_alias(pattern, track, source)?;
        self.commit(candidate)
    }
    /// Snapshot resolved content into the target block as one undoable edit.
    pub fn make_track_unique(&mut self, pattern: PatternId, track: TrackId) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.make_track_unique(pattern, track)?;
        self.commit(candidate)
    }
    pub fn set_occurrence_track_mute(
        &mut self,
        occurrence: usize,
        track: TrackId,
        muted: bool,
    ) -> Result<(), String> {
        self.protect()?;
        let mut candidate = self.song.clone();
        candidate.set_occurrence_track_mute(occurrence, track, muted)?;
        self.commit(candidate)
    }
    pub fn move_occurrence(&mut self, delta: i32) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let Some(target) = self.occurrence.checked_add_signed(delta as isize) else {
            return Ok(());
        };
        if target >= self.song.order.len() {
            return Ok(());
        }
        let mut candidate = self.song.clone();
        candidate.move_occurrence(self.occurrence, target)?;
        self.commit(candidate)?;
        self.occurrence = target;
        Ok(())
    }
    /// Remove only the selected order entry, never its shared pattern content.
    /// Retain at least one occurrence for this interactive operation. Select the
    /// next entry at the same index, or the previous entry when removing the last.
    pub fn remove_occurrence(&mut self) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        if self.song.order.len() <= 1 {
            return Err("Cannot remove the final occurrence; keep at least one order entry".into());
        }
        let mut candidate = self.song.clone();
        candidate.order.remove(self.occurrence);
        self.apply_document_edit(candidate)
    }

    fn checked_pattern_id(&self) -> Result<PatternId, String> {
        let id = self
            .song
            .order
            .get(self.occurrence)
            .ok_or("No selected occurrence")?
            .pattern;
        let pattern = self
            .song
            .patterns
            .get(&id)
            .ok_or("Unknown selected pattern")?;
        if !(1..=65536).contains(&pattern.rows) {
            return Err("Invalid selected pattern length".into());
        }
        Ok(id)
    }
    fn checked_cursor(&self) -> Result<PatternId, String> {
        let id = self.checked_pattern_id()?;
        let track = self
            .song
            .tracks
            .get(&self.track)
            .ok_or("No selected track")?;
        if !self.song.track_order.contains(&self.track)
            || self.row >= self.song.patterns[&id].rows
            || self.column >= track.note_columns
        {
            return Err("Cursor is outside the selected pattern or track".into());
        }
        Ok(id)
    }
    /// Requires a valid selected occurrence; UI callers first check `has_grid`.
    pub fn pattern_id(&self) -> PatternId {
        self.song.order[self.occurrence].pattern
    }
    pub fn pattern(&self) -> &Pattern {
        &self.song.patterns[&self.pattern_id()]
    }
    /// Terminal displayed block identity. Reading an alias does not authorize
    /// writing its source: navigate explicitly or make the selected block unique.
    pub fn content_source(&self) -> Result<(PatternId, TrackId), String> {
        let pattern = self.checked_cursor()?;
        self.song.resolve_track_source(pattern, self.track)
    }
    pub fn cell(&self) -> Cell {
        self.song
            .order
            .get(self.occurrence)
            .and_then(|o| {
                self.song
                    .resolved_track(o.pattern, self.track)
                    .ok()
                    .flatten()
            })
            .and_then(|r| r.get(&self.row))
            .and_then(|c| c.get(&self.column))
            .cloned()
            .unwrap_or_default()
    }
    pub fn select(&mut self, occurrence: usize, track: TrackId) -> Result<(), String> {
        let pattern = self
            .song
            .order
            .get(occurrence)
            .and_then(|o| self.song.patterns.get(&o.pattern))
            .ok_or("Invalid occurrence or pattern")?;
        let columns = self
            .song
            .tracks
            .get(&track)
            .ok_or("Invalid track")?
            .note_columns;
        if !(1..=65536).contains(&pattern.rows)
            || !(1..=256).contains(&columns)
            || !self.song.track_order.contains(&track)
        {
            return Err("Invalid pattern length or track columns".into());
        }
        let row = self.row.min(pattern.rows - 1);
        let column = self.column.min(columns - 1);
        self.occurrence = occurrence;
        self.track = track;
        self.row = row;
        self.column = column;
        Ok(())
    }
    fn protect(&self) -> Result<(), String> {
        if self.edit_mode {
            Ok(())
        } else {
            Err("Enable Edit Mode to change song content".into())
        }
    }
    pub fn set_cell(&mut self, cell: Cell) -> Result<(), String> {
        self.protect()?;
        let pattern_id = self.checked_cursor()?;
        self.song.require_local_track(pattern_id, self.track)?;
        let mut candidate = self.song.clone();
        let rows = &mut candidate.patterns.get_mut(&pattern_id).unwrap().tracks;
        if cell == Cell::default() {
            if let Some(track) = rows.get_mut(&self.track) {
                if let Some(row) = track.get_mut(&self.row) {
                    row.remove(&self.column);
                    if row.is_empty() {
                        track.remove(&self.row);
                    }
                }
                if track.is_empty() {
                    rows.remove(&self.track);
                }
            }
        } else {
            rows.entry(self.track)
                .or_default()
                .entry(self.row)
                .or_default()
                .insert(self.column, cell);
        }
        self.commit(candidate)
    }
    /// Replace only the selected cell's step commands as one validated transaction.
    /// Empty effects clear commands without clearing the note or independent fields.
    pub fn set_effects(&mut self, effects: StepEffects) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let mut cell = self.cell();
        cell.effects = effects;
        self.set_cell(cell)
    }
    pub fn enter_note(&mut self, note: Note) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let mut cell = self.cell();
        cell.note = note;
        self.set_cell(cell)?;
        self.move_rows(self.edit_step as i64);
        Ok(())
    }
    pub fn clear(&mut self, whole_cell: bool) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let mut cell = self.cell();
        if whole_cell {
            cell = Cell::default();
        } else {
            match self.field {
                Field::Note => cell.note = Note::Empty,
                Field::Instrument => cell.instrument = None,
                Field::Velocity => cell.velocity = None,
                Field::Pan => cell.pan = None,
                Field::Delay => cell.delay = None,
            }
        }
        self.set_cell(cell)
    }
    /// Explicit typed entry; blank removes only this field. Numeric values use decimal.
    pub fn enter_value(&mut self, text: &str) -> Result<(), String> {
        self.protect()?;
        self.checked_cursor()?;
        let text = text.trim();
        let mut cell = self.cell();
        match self.field {
            Field::Note => {
                cell.note = if text.is_empty() {
                    Note::Empty
                } else if text.eq_ignore_ascii_case("off") {
                    Note::Off
                } else {
                    Note::Note(
                        text.parse()
                            .map_err(|_| "Enter MIDI pitch 0..127, OFF or blank")?,
                    )
                }
            }
            Field::Instrument => {
                cell.instrument = if text.is_empty() {
                    None
                } else {
                    Some(InstrumentId(
                        text.parse()
                            .map_err(|_| "Enter an instrument ID or blank")?,
                    ))
                }
            }
            Field::Velocity => {
                cell.velocity = if text.is_empty() {
                    None
                } else {
                    Some(text.parse().map_err(|_| "Enter velocity 0..1 or blank")?)
                }
            }
            Field::Pan => {
                cell.pan = if text.is_empty() {
                    None
                } else {
                    Some(text.parse().map_err(|_| "Enter pan -1..1 or blank")?)
                }
            }
            Field::Delay => {
                cell.delay = if text.is_empty() {
                    None
                } else {
                    Some(text.parse().map_err(|_| "Enter row delay 0..<1 or blank")?)
                }
            }
        }
        self.set_cell(cell)
    }
    pub fn move_rows(&mut self, amount: i64) {
        if let Ok(id) = self.checked_pattern_id() {
            self.row = (self.row as i64)
                .saturating_add(amount)
                .clamp(0, self.song.patterns[&id].rows as i64 - 1) as u32;
        }
    }
    pub fn move_field(&mut self, amount: i32) {
        if self.checked_pattern_id().is_err() {
            return;
        }
        let mut columns = Vec::new();
        for track in &self.song.track_order {
            let Some(spec) = self.song.tracks.get(track) else {
                continue;
            };
            for column in 0..spec.note_columns {
                columns.push((*track, column));
            }
        }
        let Some(index) = columns.iter().position(|c| *c == (self.track, self.column)) else {
            return;
        };
        let next = (index as i64 * 5 + self.field.index() as i64 + amount as i64)
            .clamp(0, columns.len() as i64 * 5 - 1) as usize;
        (self.track, self.column) = columns[next / 5];
        self.field = Field::ALL[next % 5];
    }
    pub fn resize(&mut self, rows: u32) -> Result<(), String> {
        self.protect()?;
        let pattern_id = self.checked_cursor()?;
        let mut candidate = self.song.clone();
        candidate.patterns.get_mut(&pattern_id).unwrap().rows = rows;
        self.commit(candidate)
            .map_err(|e| format!("Cannot resize: {e}. Clear occupied rows before shrinking."))?;
        self.row = self.row.min(rows - 1);
        Ok(())
    }
    pub fn append(&mut self, independent: bool) -> Result<(), String> {
        self.protect()?;
        let source = self.checked_cursor()?;
        let mut candidate = self.song.clone();
        let id = if independent {
            let id = PatternId(
                candidate
                    .patterns
                    .keys()
                    .map(|p| p.0)
                    .max()
                    .unwrap_or(0)
                    .checked_add(1)
                    .ok_or("Pattern ID space exhausted")?,
            );
            candidate.clone_pattern(source, id)?;
            candidate
                .patterns
                .get_mut(&id)
                .unwrap()
                .name
                .push_str(" copy");
            id
        } else {
            source
        };
        candidate.append_repeat(id)?;
        self.commit(candidate)?;
        self.select(self.song.order.len() - 1, self.track)
    }
}

pub fn note_label(note: Note) -> String {
    match note {
        Note::Empty => "---".into(),
        Note::Off => "OFF".into(),
        Note::Note(n) => {
            let names = [
                "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B",
            ];
            format!("{}{}", names[n as usize % 12], n as i16 / 12 - 1)
        }
    }
}
