//! Accepted marker intervals -> a fresh, ordinary drum bank. This is a pure
//! metadata transaction, not a slicer, decoder, renderer or tempo compiler.
//! Preview/commit do not verify source bytes. Adapters must authorize and freshly
//! hash the selected source off-thread before their revision-guarded commit.
use crate::song::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

pub const VERSION: u32 = 1;
pub const ALGORITHM: &str = "accepted-marker-bank-v1";
pub const MAX_INTERVALS: usize = 16;
pub const MAX_VISITS: usize = 65_536;
pub const MAX_PHRASE_CELLS: usize = 32_768;
pub const MAX_VALIDATION_UNITS: usize = 1_048_576;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct MarkerBankConfig {
    pub version: u32,
    pub marker_set: MarkerSetId,
    /// Strictly ascending, unique indices. Index i means exactly
    /// [markers[i], markers[i+1]); sparse selections are permitted.
    pub interval_indices: Vec<u16>,
    /// Used for the fresh kit and instrument. Per-pad names are deterministic.
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarkerBankPad {
    pub interval_index: u16,
    pub pad: PadId,
    pub region: RegionId,
    pub slot: u8,
    pub selector: u8,
    pub root_note: u8,
    pub name: String,
    pub start_frame: u64,
    pub end_frame: u64,
    pub tune_semitones: i8,
    pub gain: f32,
    pub pan: f32,
    pub mode: PadMode,
    pub new_note: PadNewNote,
    pub choke_group: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct MarkerBankWork {
    pub ordered_track_blocks: usize,
    pub ordered_rows: usize,
    pub ordered_cells: usize,
    /// Resolved definitions, including unused patterns and repeated alias reads.
    pub definition_rows: usize,
    pub definition_cells: usize,
    pub phrase_cells: usize,
    pub prospective_instruments: usize,
    /// (definition_cells + ordered_cells + phrase_cells) * prospective_instruments.
    /// A conservative local validation-work bound, not a latency measurement.
    pub validation_units: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MarkerBankSummary {
    pub version: u32,
    pub algorithm: String,
    pub marker_set: MarkerSetId,
    pub source: SampleId,
    pub source_sha256: String,
    pub source_sample_rate: u32,
    pub source_channels: u16,
    pub source_frames: u64,
    pub kit: DrumKitId,
    pub instrument: InstrumentId,
    pub name: String,
    pub pads: Vec<MarkerBankPad>,
    pub work: MarkerBankWork,
    /// Always false: this pure plan binds stored metadata, not external bytes.
    pub source_identity_verified: bool,
    pub warnings: Vec<String>,
}

/// Private candidate/base authority; no deserialization or caller replacement.
/// Controllers additionally guard their revision/owner to reject ABA and Open.
#[derive(Debug, Clone)]
pub struct MarkerBankPreview {
    id: String,
    base_sha256: String,
    candidate_sha256: String,
    base: Song,
    candidate: Song,
    config: MarkerBankConfig,
    summary: MarkerBankSummary,
}
impl MarkerBankPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &MarkerBankConfig {
        &self.config
    }
    pub fn summary(&self) -> &MarkerBankSummary {
        &self.summary
    }
    pub fn source(&self) -> &SampleAsset {
        &self.base.samples[&self.summary.source]
    }
    pub fn base_sha256(&self) -> &str {
        &self.base_sha256
    }
    pub fn candidate_sha256(&self) -> &str {
        &self.candidate_sha256
    }
    pub fn is_current(&self, current: &Song) -> Result<bool, String> {
        // PartialEq alone treats +0.0 and -0.0 as equal. Preserve exact stored
        // scalar representations, including fields unrelated to this bank.
        Ok(current == &self.base && hash(current)? == self.base_sha256)
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if !self.is_current(current)? {
            return Err("Marker bank preview is stale; prepare and review again".into());
        }
        Ok(self.candidate.clone())
    }
}

fn hash(value: &impl Serialize) -> Result<String, String> {
    serde_json::to_vec(value)
        .map(|bytes| crate::sample_asset::content_sha256(&bytes))
        .map_err(|e| e.to_string())
}

fn validate_config(config: &MarkerBankConfig) -> Result<(), String> {
    if config.version != VERSION {
        return Err("Marker bank requires version 1".into());
    }
    if config.interval_indices.is_empty()
        || config.interval_indices.len() > MAX_INTERVALS
        || config
            .interval_indices
            .windows(2)
            .any(|pair| pair[0] >= pair[1])
    {
        return Err("Select 1..16 strictly ascending, unique marker interval indices; no sorting, merging or truncation is performed".into());
    }
    if config.name.trim().is_empty() || config.name.len() > 256 {
        return Err("Marker bank name must be nonblank and at most 256 UTF-8 bytes".into());
    }
    Ok(())
}

/// Runs before full model validation or any Song clone. New kit metadata can
/// activate the existing conservative incoming-instrument validator even when
/// the kit is not used. Bound all definitions, not just ordered playback.
fn check_work(song: &Song) -> Result<MarkerBankWork, String> {
    crate::generation::check_work(song, false)?;
    let mut terminals = BTreeMap::new();
    let mut patterns = BTreeMap::new();
    let (mut definition_rows, mut definition_cells) = (0usize, 0usize);
    for (&id, pattern) in &song.patterns {
        let (mut rows, mut cells) = (0usize, 0usize);
        for &track in pattern.tracks.keys().chain(pattern.track_aliases.keys()) {
            let terminal = song.resolve_track_source(id, track)?;
            let (add_rows, add_cells) = *terminals.entry(terminal).or_insert_with(|| {
                song.patterns[&terminal.0]
                    .tracks
                    .get(&terminal.1)
                    .map_or((0, 0), |rows| {
                        (
                            rows.len(),
                            rows.values()
                                .fold(0usize, |n, row| n.saturating_add(row.len())),
                        )
                    })
            });
            rows = rows.saturating_add(add_rows);
            cells = cells.saturating_add(add_cells);
        }
        definition_rows = definition_rows.saturating_add(rows);
        definition_cells = definition_cells.saturating_add(cells);
        if definition_rows > MAX_VISITS || definition_cells > MAX_VISITS {
            return Err("Marker bank exceeds 65536 resolved definition rows or cells, including unused patterns, aliases and default cells; reduce stored content before building a bank".into());
        }
        patterns.insert(id, (rows, cells));
    }
    let (mut ordered_rows, mut ordered_cells) = (0usize, 0usize);
    for occurrence in &song.order {
        let &(rows, cells) = patterns
            .get(&occurrence.pattern)
            .ok_or("Marker bank references unknown ordered pattern")?;
        ordered_rows = ordered_rows.saturating_add(rows);
        ordered_cells = ordered_cells.saturating_add(cells);
    }
    let mut phrase_cells = 0usize;
    for phrase in song.phrases.values() {
        for row in phrase.cells.values() {
            phrase_cells = phrase_cells.saturating_add(row.len());
            if phrase_cells > MAX_PHRASE_CELLS {
                return Err("Marker bank exceeds 32768 phrase-definition cells".into());
            }
        }
    }
    let prospective_instruments = song
        .instruments
        .len()
        .checked_add(1)
        .ok_or("Marker bank prospective instrument count overflow")?;
    let validation_units = definition_cells
        .checked_add(ordered_cells)
        .and_then(|n| n.checked_add(phrase_cells))
        .and_then(|n| n.checked_mul(prospective_instruments))
        .ok_or("Marker bank prospective validation work overflow")?;
    if validation_units > MAX_VALIDATION_UNITS {
        return Err(format!("Marker bank prospective validation work {validation_units} exceeds {MAX_VALIDATION_UNITS} units; reduce stored cells, repeats or instrument definitions before building a bank"));
    }
    Ok(MarkerBankWork {
        ordered_track_blocks: song.order.len() * song.track_order.len(),
        ordered_rows,
        ordered_cells,
        definition_rows,
        definition_cells,
        phrase_cells,
        prospective_instruments,
        validation_units,
    })
}

/// Build a bounded, nonmutating metadata plan from persisted accepted markers.
/// No source BPM, source assignment for existing notes, playback evaluation,
/// path resolution, file access, decoder or audition is required or performed.
pub fn preview(song: &Song, config: &MarkerBankConfig) -> Result<MarkerBankPreview, String> {
    validate_config(config)?;
    crate::project::check_serialized_size(song)?;
    let work = check_work(song)?;
    // Validate BEFORE allocation: an explicit unresolved instrument/region/kit
    // reference must not become valid merely because max+1 would fill that ID.
    song.validate()?;
    let set = song
        .marker_sets
        .get(&config.marker_set)
        .ok_or("Unknown accepted marker set; save/review markers first")?;
    for &index in &config.interval_indices {
        if usize::from(index) + 1 >= set.markers.len() {
            return Err(format!("Marker interval {index} has no adjacent end boundary in the selected saved marker set"));
        }
    }
    let source = &song.samples[&set.sample];
    if song.regions.len() + config.interval_indices.len() > 4096 {
        return Err("Marker bank would exceed 4096 regions".into());
    }
    if song.drum_kits.len() >= 128 {
        return Err("Marker bank would exceed 128 drum kits".into());
    }
    let previous_region = song.regions.keys().next_back().map_or(0, |id| id.0);
    previous_region
        .checked_add(config.interval_indices.len() as u64)
        .ok_or("Marker bank region ID space exhausted; no holes are reused")?;
    let kit = DrumKitId(
        song.drum_kits
            .keys()
            .next_back()
            .map_or(0, |id| id.0)
            .checked_add(1)
            .ok_or("Marker bank kit ID space exhausted; no holes are reused")?,
    );
    let instrument = InstrumentId(
        song.instruments
            .keys()
            .next_back()
            .map_or(0, |id| id.0)
            .checked_add(1)
            .ok_or("Marker bank instrument ID space exhausted; no holes are reused")?,
    );
    let pads: Vec<_> = config
        .interval_indices
        .iter()
        .enumerate()
        .map(|(slot, &index)| MarkerBankPad {
            interval_index: index,
            pad: PadId(slot as u64 + 1),
            region: RegionId(previous_region + slot as u64 + 1),
            slot: slot as u8,
            selector: 36 + slot as u8,
            root_note: 60,
            name: format!("Slice {:03}", u32::from(index) + 1),
            start_frame: set.markers[usize::from(index)],
            end_frame: set.markers[usize::from(index) + 1],
            tune_semitones: 0,
            gain: 1.0,
            pan: 0.0,
            mode: PadMode::OneShot,
            new_note: PadNewNote::Overlap,
            choke_group: None,
        })
        .collect();
    // One full candidate clone and one final validation, not N transactional
    // upserts that each clone/validate the entire document.
    let mut candidate = song.clone();
    let mut mapped = BTreeMap::new();
    for pad in &pads {
        candidate.regions.insert(
            pad.region,
            SampleRegion {
                name: pad.name.clone(),
                sample: set.sample,
                start_frame: pad.start_frame,
                end_frame: pad.end_frame,
                root_note: pad.root_note,
                amp_envelope: None,
                forward_loop: None,
            },
        );
        mapped.insert(
            pad.pad,
            DrumPad {
                slot: pad.slot,
                name: pad.name.clone(),
                note: pad.selector,
                region: Some(pad.region),
                velocity_zones: vec![],
                root_note: pad.root_note,
                tune_semitones: pad.tune_semitones,
                gain: pad.gain,
                pan: pad.pan,
                mode: pad.mode,
                new_note: pad.new_note,
                choke_group: pad.choke_group,
            },
        );
    }
    candidate.drum_kits.insert(
        kit,
        DrumKitInstrument {
            name: config.name.clone(),
            pads: mapped,
        },
    );
    candidate.instruments.insert(
        instrument,
        Instrument {
            name: config.name.clone(),
            kind: InstrumentKind::DrumKit { kit },
            sample: None,
            region: None,
        },
    );
    crate::project::check_serialized_size(&candidate)?;
    candidate.validate()?;
    let summary = MarkerBankSummary {
        version: VERSION, algorithm: ALGORITHM.into(), marker_set: config.marker_set,
        source: set.sample, source_sha256: source.sha256.clone(),
        source_sample_rate: source.sample_rate, source_channels: source.channels,
        source_frames: source.frames, kit, instrument, name: config.name.clone(), pads, work,
        source_identity_verified: false,
        warnings: vec![
            "Pure metadata preview; selected source bytes are not verified here. Native/MCP Create must authorize and freshly hash that source before the guarded commit; no audio decoding is performed.".into(),
            "Root 60 is a new native-pitch reference, not detected pitch. Selectors 36..51, unity gain, center pan, OneShot/Overlap; no choke, velocity layers, ADSR or loops are inferred.".into(),
            "Only new regions, one new kit and its new instrument are added. Existing clocks, patterns, order, markers, source audio, references and reviewed roles remain unchanged; no playback starts automatically.".into(),
            "Regions retain exact adjacent [start,end) source bounds, including any tail before the next marker. No extraction, resampling, fades, minimum-length inference or timing compilation.".into(),
            "This is not a live marker recipe: later marker edits do not regenerate the bank. Shared source audio remains immutable; region metadata is ordinarily editable.".into(),
            "The local 65536-visit and 1048576 prospective-validation-unit limits are conservative resource guards, not realtime or full-renderability guarantees.".into(),
        ],
    };
    let base_sha256 = hash(song)?;
    let candidate_sha256 = hash(&candidate)?;
    let id = hash(&(ALGORITHM, &base_sha256, config, &candidate_sha256))?;
    Ok(MarkerBankPreview {
        id,
        base_sha256,
        candidate_sha256,
        base: song.clone(),
        candidate,
        config: config.clone(),
        summary,
    })
}
