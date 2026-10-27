//! Pure, bounded audition snapshots for the existing Song Engine/transport.
//! No device, file access, live event injection, document edits or second renderer.
use crate::{
    song::*,
    song_render::{self, Command, RenderLayerSelection, RenderPad, RenderRegion},
    RATE, TAIL,
};
use serde::Serialize;
use std::{collections::BTreeMap, path::PathBuf};

pub const VERSION: u32 = 1;
pub const MUSICAL_FRAMES: u64 = 96_000;
pub const TOTAL_FRAMES: u64 = MUSICAL_FRAMES + TAIL;
pub const MAX_PATH_BYTES: usize = 8192;
pub const MAX_METADATA_BYTES: usize = 256 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum AuditionTarget {
    /// Source/region IDs in the synthetic Song are local, not registrations.
    RegionDraft,
    Pad {
        instrument: InstrumentId,
        kit: DrumKitId,
        pad: PadId,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AuditionSummary {
    pub version: u32,
    pub target: AuditionTarget,
    pub sample_rate: u32,
    pub musical_frames: u64,
    pub tail_frames: u64,
    pub total_frames: u64,
    pub input_velocity: f32,
    pub velocity_bin: u8,
    pub sounding_note: u8,
    /// Evaluator ON gain before the unchanged Engine master and gain smoothing.
    pub on_gain: f32,
    pub pan: f32,
    pub master_gain: f32,
    /// Original absolute lexical path, for authorization before canonicalization.
    pub source_path: PathBuf,
    pub region: RenderRegion,
    /// Exact committed pad controls and chosen source; absent for region drafts.
    pub pad: Option<RenderPad>,
    pub layer_selection: Option<RenderLayerSelection>,
    pub retained_regions: usize,
    pub retained_sources: usize,
    /// Prepare checks model metadata only. Engine loading later verifies bytes.
    pub source_identity_verified: bool,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AuditionPreview {
    id: String,
    song: Song,
    summary: AuditionSummary,
}
impl AuditionPreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    /// Immutable synthetic snapshot, usable with the existing Engine, Producer,
    /// host Transport, or offline export. It is never an Editor candidate.
    pub fn song(&self) -> &Song {
        &self.song
    }
    pub fn summary(&self) -> &AuditionSummary {
        &self.summary
    }
}

fn source_shape(asset: &SampleAsset, region: &SampleRegion) -> Result<(), String> {
    if asset.path.as_os_str().len() > MAX_PATH_BYTES
        || asset.sha256.len() != 64
        || region.name.len() > 256
    {
        return Err("Audition requires source paths at most 8192 bytes, SHA-256 metadata, and region names at most 256 bytes".into());
    }
    Ok(())
}

fn skeleton(instrument: InstrumentId, note: u8) -> Song {
    Song {
        schema: 1,
        generation_recipes: Default::default(),
        title: "Read-only audition".into(),
        bpm: 120.0,
        lines_per_beat: 4,
        tracks: BTreeMap::from([(
            TrackId(1),
            Track {
                name: "Audition".into(),
                note_columns: 1,
            },
        )]),
        track_order: vec![TrackId(1)],
        instruments: BTreeMap::new(),
        patterns: BTreeMap::from([(
            PatternId(1),
            Pattern {
                generation: None,
                track_aliases: BTreeMap::new(),
                name: "Audition".into(),
                rows: 16,
                tracks: BTreeMap::from([(
                    TrackId(1),
                    BTreeMap::from([(
                        0,
                        BTreeMap::from([(
                            0,
                            Cell {
                                note: Note::Note(note),
                                instrument: Some(instrument),
                                velocity: Some(1.0),
                                pan: Some(0.0),
                                ..Cell::default()
                            },
                        )]),
                    )]),
                )]),
            },
        )]),
        phrases: BTreeMap::new(),
        samples: BTreeMap::new(),
        regions: BTreeMap::new(),
        marker_sets: BTreeMap::new(),
        drum_kits: BTreeMap::new(),
        grooves: BTreeMap::new(),
        multisamples: BTreeMap::new(),
        region_catalog: BTreeMap::new(),
        track_mix: BTreeMap::new(),
        order: vec![Occurrence {
            muted_tracks: Default::default(),
            section: None,
            pattern: PatternId(1),
        }],
    }
}

/// Audition the exact draft audio bounds/root/envelope/loop without Apply or
/// source registration. The supplied source must come from an authorized import
/// or registered asset; this pure function does not verify external bytes.
pub fn region_preview(
    asset: &SampleAsset,
    draft: &SampleRegion,
) -> Result<AuditionPreview, String> {
    source_shape(asset, draft)?;
    let mut song = skeleton(InstrumentId(1), draft.root_note);
    song.samples.insert(SampleId(1), asset.clone());
    let mut region = draft.clone();
    region.sample = SampleId(1);
    song.regions.insert(RegionId(1), region);
    song.instruments.insert(
        InstrumentId(1),
        Instrument {
            name: "Audition region".into(),
            kind: InstrumentKind::Pitched,
            sample: None,
            region: Some(RegionId(1)),
        },
    );
    finish(song, AuditionTarget::RegionDraft)
}

/// Audition one committed pad; unrelated arrangement, instruments, pads and
/// assets are not copied or evaluated. Preserve the complete selected pad's
/// mapping (including unused alternatives) for ordinary model validation and
/// exact evaluator layer provenance. Only the chosen source is later decoded.
pub fn pad_preview(
    base: &Song,
    instrument: InstrumentId,
    pad: PadId,
) -> Result<AuditionPreview, String> {
    let definition = base
        .instruments
        .get(&instrument)
        .ok_or("Unknown audition instrument")?;
    let InstrumentKind::DrumKit { kit } = definition.kind else {
        return Err("Pad audition requires a drum-kit instrument".into());
    };
    let kit_definition = base.drum_kits.get(&kit).ok_or("Unknown audition kit")?;
    let pad_definition = kit_definition
        .pads
        .get(&pad)
        .ok_or("Unknown audition pad")?;
    // Check all variable-size metadata BEFORE flattening/cloning. Do not impose
    // a new full-Song evaluator policy or traverse unrelated empty blocks.
    if definition.name.len() > 256
        || definition.sample.is_some()
        || definition.region.is_some()
        || kit_definition.name.len() > 256
        || pad_definition.name.len() > 128
        || pad_definition.velocity_zones.len() > 8
        || pad_definition
            .velocity_zones
            .iter()
            .any(|zone| zone.alternatives.is_empty() || zone.alternatives.len() > 8)
        || pad_definition
            .velocity_zones
            .iter()
            .map(|zone| zone.alternatives.len())
            .sum::<usize>()
            > 32
    {
        return Err("Audition pad metadata exceeds bounded names/8 zones/8 alternatives per zone/32 alternatives, or has conflicting legacy source fields".into());
    }
    let mut song = skeleton(instrument, pad_definition.note);
    for id in pad_definition.source_regions() {
        let region = base
            .regions
            .get(&id)
            .ok_or("Audition pad references unknown region")?;
        let source = base
            .samples
            .get(&region.sample)
            .ok_or("Audition region references unknown source")?;
        source_shape(source, region)?;
        song.regions.insert(id, region.clone());
        song.samples.insert(region.sample, source.clone());
    }
    song.instruments.insert(instrument, definition.clone());
    song.drum_kits.insert(
        kit,
        DrumKitInstrument {
            name: kit_definition.name.clone(),
            pads: BTreeMap::from([(pad, pad_definition.clone())]),
        },
    );
    finish(
        song,
        AuditionTarget::Pad {
            instrument,
            kit,
            pad,
        },
    )
}

fn finish(song: Song, target: AuditionTarget) -> Result<AuditionPreview, String> {
    if crate::project::check_serialized_size(&song)? > MAX_METADATA_BYTES {
        return Err("Audition snapshot exceeds 256 KiB metadata budget".into());
    }
    let manifest = song_render::evaluate_model(&song)?;
    let starts: Vec<_> = manifest
        .events
        .iter()
        .filter(|event| matches!(event.command, Command::On { .. }))
        .collect();
    if starts.len() != 1
        || manifest.events.len() != 2
        || manifest.musical_frames != MUSICAL_FRAMES
        || manifest.tail_frames != TAIL
        || manifest.sample_rate != RATE
    {
        return Err("Audition must contain exactly one bounded ON and final release".into());
    }
    let start = starts[0];
    let Command::On {
        voice,
        pitch,
        volume,
        pan,
    } = start.command
    else {
        unreachable!()
    };
    let (path, region, layer_selection) = if let Some(pad) = manifest.pad_voices.get(&voice) {
        (
            pad.path.clone(),
            pad.region.clone(),
            pad.layer_selection.clone(),
        )
    } else {
        (
            manifest.assets[&start.instrument].clone(),
            manifest.regions[&start.instrument].clone(),
            None,
        )
    };
    if !path.is_absolute() {
        return Err("Audition source path must be absolute; load the saved project or import/register the source first (no process-CWD resolution)".into());
    }
    let summary = AuditionSummary {
        version: VERSION,
        target,
        sample_rate: RATE,
        musical_frames: MUSICAL_FRAMES,
        tail_frames: TAIL,
        total_frames: TOTAL_FRAMES,
        input_velocity: 1.0,
        velocity_bin: 127,
        sounding_note: pitch,
        on_gain: volume,
        pan,
        master_gain: manifest.master_gain,
        source_path: path,
        region,
        pad: manifest.pad_voices.get(&voice).cloned(),
        layer_selection,
        retained_regions: song.regions.len(),
        retained_sources: song.samples.len(),
        source_identity_verified: false,
        warnings: vec![
            "Read-only cold audition: one ON, fixed 2 s musical gate plus 0.5 s tail at 48 kHz; long regions/loops are deliberately released at the gate. Natural source exhaustion may be earlier.".into(),
            "Unity input velocity/bin 127; region draft plays at its native root. Pad uses committed gain/pan/tune and first RR alternative in the selected zone; no neighbouring pad/choke or arrangement context, no cycling through other alternatives.".into(),
            "Preparation validates stored metadata only, not source bytes. The existing Engine verifies and decodes the chosen source off the callback before playback. Unused mapped alternatives are not decoded.".into(),
            "When explicitly played by the native desktop: exclusive output session, not mixed with Song playback. Explicit Stop/focus loss/closing cancels with a hard stop that may click; already submitted device buffers cannot be retracted. Mouse release is not a gated-note command.".into(),
        ],
    };
    let id =
        crate::sample_asset::content_sha256(&serde_json::to_vec(&song).map_err(|e| e.to_string())?);
    Ok(AuditionPreview { id, song, summary })
}
