//! Pure, bounded source-frame -> ordinary tracker document compiler.
//! No decoder, file IO, UI clock, random generator, or second playback engine.
pub(crate) mod timing;
use crate::{song::*, RATE};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use timing::Rational;
pub const ALGORITHM: &str = "source-frame-compiler-v1";
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Subdivision {
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
    OneTwentyEighth,
}
impl Subdivision {
    pub fn denominator(self) -> u32 {
        match self {
            Self::Sixteenth => 16,
            Self::ThirtySecond => 32,
            Self::SixtyFourth => 64,
            Self::OneTwentyEighth => 128,
        }
    }
    fn beats(self, count: u64) -> Rational {
        Rational::new(u128::from(count) * 4, u128::from(self.denominator()))
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub enum CompileTiming {
    SourceRhythm,
    Grid { division: Subdivision },
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileRoll {
    pub division: Subdivision,
    pub hits: u8,
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TailPolicy {
    Overlap,
    ReleasePrevious,
    CutPrevious,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CompileConfig {
    pub marker_set: MarkerSetId,
    pub target_bpm: f64,
    pub timing: CompileTiming,
    pub cycles: u16,
    pub roll: Option<CompileRoll>,
    pub tail: TailPolicy,
    pub minimum_slice_frames: u64,
    pub root_note: u8,
    pub pad_tails: bool,
    pub name: String,
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CompileSummary {
    pub algorithm: String,
    pub marker_set: MarkerSetId,
    pub source: SampleId,
    pub source_sha256: String,
    pub source_bpm: f64,
    pub previous_bpm: f64,
    pub target_bpm: f64,
    pub pattern: PatternId,
    pub track: TrackId,
    pub occurrence: usize,
    pub regions: Vec<RegionId>,
    pub instruments: Vec<InstrumentId>,
    pub slice_count: u16,
    pub note_starts: u32,
    pub note_columns: u16,
    pub sequence_rows: u32,
    pub tail_padding_rows: u32,
    pub pattern_rows: u32,
    pub warnings: Vec<String>,
}
/// Opaque plan: callers cannot replace its exact candidate or expected base.
/// Controller must additionally guard its revision and verify source bytes off-thread.
#[derive(Debug, Clone)]
pub struct CompilePreview {
    id: String,
    base: Song,
    candidate: Song,
    config: CompileConfig,
    summary: CompileSummary,
}
impl CompilePreview {
    pub fn id(&self) -> &str {
        &self.id
    }
    pub fn base(&self) -> &Song {
        &self.base
    }
    pub fn candidate(&self) -> &Song {
        &self.candidate
    }
    pub fn config(&self) -> &CompileConfig {
        &self.config
    }
    pub fn summary(&self) -> &CompileSummary {
        &self.summary
    }
    pub fn source(&self) -> &SampleAsset {
        &self.base.samples[&self.summary.source]
    }
    pub fn commit(&self, current: &Song) -> Result<Song, String> {
        if current != &self.base {
            return Err("Compiler preview is stale; prepare a new preview".into());
        }
        Ok(self.candidate.clone())
    }
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CleanupReason {
    NearStart,
    NearEnd,
    TooClose,
    Duplicate,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RemovedMarker {
    pub frame: u64,
    pub reason: CleanupReason,
}
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarkerCleanup {
    pub markers: Vec<u64>,
    pub removed: Vec<RemovedMarker>,
    pub minimum_slice_frames: u64,
}
/// A review proposal only. Never modifies a saved marker set or silently snaps attacks.
pub fn suggest_marker_cleanup(
    source: &SampleAsset,
    candidates: &[u64],
    minimum_slice_frames: u64,
) -> Result<MarkerCleanup, String> {
    if minimum_slice_frames == 0 || minimum_slice_frames > source.frames {
        return Err("Minimum slice length must be 1..=source frames".into());
    }
    if candidates.len() > 65536 {
        return Err("Cleanup accepts at most 65536 candidate markers".into());
    }
    if candidates.iter().any(|frame| *frame > source.frames) {
        return Err("Candidate marker outside source frames".into());
    }
    let mut sorted = candidates.to_vec();
    sorted.sort_unstable();
    let mut markers = vec![0];
    let mut removed = Vec::new();
    let mut previous = None;
    for frame in sorted {
        let reason = if previous == Some(frame) {
            Some(CleanupReason::Duplicate)
        } else if frame == 0 || frame == source.frames {
            previous = Some(frame);
            continue;
        } else if frame < minimum_slice_frames {
            Some(CleanupReason::NearStart)
        } else if source.frames - frame < minimum_slice_frames {
            Some(CleanupReason::NearEnd)
        } else if frame - markers.last().unwrap() < minimum_slice_frames {
            Some(CleanupReason::TooClose)
        } else {
            None
        };
        previous = Some(frame);
        if let Some(reason) = reason {
            removed.push(RemovedMarker { frame, reason });
        } else {
            markers.push(frame);
        }
    }
    markers.push(source.frames);
    if markers.len() > 257 {
        return Err(
            "Cleanup still exceeds 257 boundaries; increase minimum spacing or edit selection"
                .into(),
        );
    }
    Ok(MarkerCleanup {
        markers,
        removed,
        minimum_slice_frames,
    })
}
fn next_id(last: Option<u64>, count: usize) -> Result<u64, String> {
    let start = last
        .unwrap_or(0)
        .checked_add(1)
        .ok_or("No fresh IDs available")?;
    start
        .checked_add(count.saturating_sub(1) as u64)
        .ok_or("No fresh IDs available")?;
    Ok(start)
}
fn existing_start_bound(song: &Song) -> Result<u64, String> {
    let mut count = 0u64;
    for occurrence in &song.order {
        for &track in &song.track_order {
            if occurrence.muted_tracks.contains(&track) {
                continue;
            }
            let Some(rows) = song.resolved_track(occurrence.pattern, track)? else {
                continue;
            };
            for cell in rows.values().flat_map(|columns| columns.values()) {
                if !matches!(cell.note, Note::Note(_)) {
                    continue;
                }
                let starts = if let Some(trigger) = &cell.phrase {
                    let phrase = &song.phrases[&trigger.phrase];
                    phrase
                        .cells
                        .values()
                        .flat_map(|columns| columns.values())
                        .filter(|cell| matches!(cell.note, Note::Note(_)))
                        .map(|cell| {
                            1 + u64::from(cell.effects.retrigger.as_ref().map_or(0, |r| r.repeats))
                        })
                        .sum::<u64>()
                        * u64::from(trigger.cycles)
                } else {
                    1 + u64::from(cell.effects.retrigger.as_ref().map_or(0, |r| r.repeats))
                };
                count = count.saturating_add(starts);
                if count > 2048 {
                    return Err("Existing arrangement exceeds conservative compiler 2048-start budget (including phrase cycles)".into());
                }
            }
        }
    }
    Ok(count)
}
struct Hit {
    beat: f64,
    slice: usize,
    row: u32,
    delay: f64,
    on_frame: u64,
    end_frame: u64,
}
struct Lane {
    available_frame: u64,
}
#[derive(Clone, Copy)]
struct Previous {
    row: u32,
    column: u16,
}
fn stop_previous(
    rows: &mut TrackPattern,
    previous: Previous,
    row: u32,
    delay: f64,
    tail: TailPolicy,
) -> Result<(), String> {
    if previous.row == row {
        let cell = rows
            .get_mut(&row)
            .unwrap()
            .get_mut(&previous.column)
            .unwrap();
        if tail == TailPolicy::CutPrevious {
            cell.effects.cut = Some(delay);
        } else {
            cell.effects.release = Some(delay);
        }
    } else {
        let cell = if tail == TailPolicy::CutPrevious {
            Cell {
                delay: Some(delay),
                effects: StepEffects {
                    cut: Some(delay),
                    ..Default::default()
                },
                ..Default::default()
            }
        } else {
            Cell {
                note: Note::Off,
                delay: Some(delay),
                ..Default::default()
            }
        };
        if rows
            .entry(row)
            .or_default()
            .insert(previous.column, cell)
            .is_some()
        {
            return Err("Compiler stop-cell collision; reduce event density".into());
        }
    }
    Ok(())
}
pub fn preview(song: &Song, config: &CompileConfig) -> Result<CompilePreview, String> {
    song.validate()?;
    if !config.target_bpm.is_finite()
        || !(20.0..=400.0).contains(&config.target_bpm)
        || !(1..=16).contains(&config.cycles)
        || config.root_note > 127
        || config.name.trim().is_empty()
        || config.name.len() > 256
    {
        return Err("Invalid compiler target BPM, cycles, root note or name".into());
    }
    let set = song
        .marker_sets
        .get(&config.marker_set)
        .ok_or("Unknown marker set")?;
    let source = &song.samples[&set.sample];
    let source_bpm = set
        .source_bpm
        .ok_or("Enter an explicit source BPM; no tempo is inferred")?;
    if config.minimum_slice_frames == 0 || config.minimum_slice_frames > source.frames {
        return Err("Minimum slice length must be 1..=source frames".into());
    }
    for (index, window) in set.markers.windows(2).enumerate() {
        if window[1] - window[0] < config.minimum_slice_frames {
            return Err(format!("Slice {} is shorter than the chosen minimum; review cleanup suggestions or explicitly lower the minimum",index+1));
        }
    }
    let slices = set.markers.len() - 1;
    let hits_per_slice = config.roll.as_ref().map_or(1, |roll| roll.hits);
    if config
        .roll
        .as_ref()
        .is_some_and(|roll| !(2..=16).contains(&roll.hits))
    {
        return Err("Roll total hits must be 2..=16".into());
    }
    let count = slices as u64 * u64::from(config.cycles) * u64::from(hits_per_slice);
    if count + existing_start_bound(song)? > 2048 {
        return Err("Compiled arrangement exceeds conservative 2048 note-start budget".into());
    }
    if song.regions.len() + slices > 4096 {
        return Err("Compiled regions exceed 4096-region budget".into());
    }
    let source_tempo = Rational::bpm(source_bpm);
    let target_tempo = Rational::bpm(config.target_bpm);
    let base_beat = |slice: usize| -> Result<Rational, String> {
        match config.timing {
            CompileTiming::SourceRhythm => Rational::new(
                u128::from(set.markers[slice]),
                u128::from(source.sample_rate) * 60,
            )
            .mul(source_tempo),
            CompileTiming::Grid { division } => Ok(division.beats(slice as u64)),
        }
    };
    let span = base_beat(slices)?;
    let rows = span
        .scale(u64::from(config.cycles) * u64::from(song.lines_per_beat))?
        .ceil()?
        .max(1);
    let sequence_rows = u32::try_from(rows)
        .ok()
        .filter(|rows| *rows <= 65536)
        .ok_or("Compiled pattern exceeds 65536 rows")?;
    let prefix_rows = song
        .order
        .iter()
        .try_fold(0u64, |sum, order| {
            sum.checked_add(u64::from(song.patterns[&order.pattern].rows))
        })
        .ok_or("Arrangement row count overflow")?;
    let per_row = f64::from(RATE) * 60.0 / (config.target_bpm * f64::from(song.lines_per_beat));
    if prefix_rows as f64 * per_row > f64::from(RATE) * 600.0 {
        return Err("Existing arrangement exceeds ten minutes at the target BPM".into());
    }
    let frame_at = |row: u32| ((prefix_rows + u64::from(row)) as f64 * per_row).round() as u64;
    let sequence_end = frame_at(sequence_rows);
    let mut hits = Vec::with_capacity(count as usize);
    for cycle in 0..config.cycles {
        let cycle_start = span.scale(u64::from(cycle))?;
        for slice in 0..slices {
            let base = cycle_start.add(base_beat(slice)?)?;
            let next = cycle_start.add(base_beat(slice + 1)?)?;
            let last = base.add(config.roll.as_ref().map_or(Rational::new(0, 1), |roll| {
                roll.division.beats(u64::from(roll.hits - 1))
            }))?;
            if !last.less(next)? {
                return Err(format!("Roll in slice {} reaches the next source/grid hit; reduce hits or choose a finer subdivision",slice+1));
            }
            for hit in 0..hits_per_slice {
                let beat = base.add(config.roll.as_ref().map_or(Rational::new(0, 1), |roll| {
                    roll.division.beats(u64::from(hit))
                }))?;
                let (row, delay, mut on_frame) = timing::encode(
                    beat,
                    prefix_rows,
                    song.lines_per_beat,
                    per_row,
                    target_tempo,
                )?;
                if !config.pad_tails {
                    on_frame = on_frame.min(sequence_end.saturating_sub(1));
                }
                let duration_frames = ((set.markers[slice + 1] - set.markers[slice])
                    * u64::from(RATE))
                .div_ceil(u64::from(source.sample_rate));
                let end_frame = on_frame
                    .checked_add(duration_frames)
                    .ok_or("Compiled frame count overflow")?;
                hits.push(Hit {
                    beat: beat.as_f64(),
                    slice,
                    row,
                    delay,
                    on_frame,
                    end_frame,
                });
            }
        }
    }
    hits.sort_by(|a, b| a.beat.total_cmp(&b.beat));
    let mut pattern_rows = sequence_rows;
    if config.pad_tails {
        let last_required_frame = if config.tail == TailPolicy::CutPrevious {
            hits.last().unwrap().end_frame
        } else {
            hits.iter().map(|hit| hit.end_frame).max().unwrap()
        };
        while frame_at(pattern_rows) < last_required_frame {
            pattern_rows = pattern_rows
                .checked_add(1)
                .filter(|rows| *rows <= 65536)
                .ok_or("Tail padding exceeds 65536 rows")?;
        }
    }
    let region_start = next_id(song.regions.keys().next_back().map(|id| id.0), slices)?;
    let instrument_start = next_id(song.instruments.keys().next_back().map(|id| id.0), slices)?;
    let pattern = PatternId(next_id(song.patterns.keys().next_back().map(|id| id.0), 1)?);
    let track = TrackId(next_id(song.tracks.keys().next_back().map(|id| id.0), 1)?);
    let mut rows = TrackPattern::new();
    let mut lanes: Vec<Lane> = Vec::new();
    let mut previous: Option<Previous> = None;
    for hit in &hits {
        let row = hit.row;
        let delay = hit.delay;
        if row >= pattern_rows {
            return Err("Compiled hit outside generated pattern".into());
        }
        if config.tail != TailPolicy::Overlap {
            if let Some(old) = previous {
                stop_previous(&mut rows, old, row, delay, config.tail)?;
                lanes[usize::from(old.column)].available_frame = hit.on_frame;
            }
        }
        let column = lanes.iter().enumerate().find_map(|(index, lane)| {
            (lane.available_frame <= hit.on_frame
                && !rows
                    .get(&row)
                    .is_some_and(|columns| columns.contains_key(&(index as u16))))
            .then_some(index)
        });
        let column = if let Some(column) = column {
            column
        } else {
            if lanes.len() >= 256 {
                return Err("Compiler density/overlap requires more than 256 note columns".into());
            }
            lanes.push(Lane { available_frame: 0 });
            lanes.len() - 1
        };
        lanes[column].available_frame = hit.end_frame;
        let cell = Cell {
            note: Note::Note(config.root_note),
            instrument: Some(InstrumentId(instrument_start + hit.slice as u64)),
            velocity: Some(1.0),
            pan: Some(0.0),
            delay: Some(delay),
            ..Default::default()
        };
        if rows
            .entry(row)
            .or_default()
            .insert(column as u16, cell)
            .is_some()
        {
            return Err("Compiler note-cell collision".into());
        }
        previous = Some(Previous {
            row,
            column: column as u16,
        });
    }
    let mut candidate = song.clone();
    candidate.bpm = config.target_bpm;
    let mut regions = Vec::with_capacity(slices);
    let mut instruments = Vec::with_capacity(slices);
    for slice in 0..slices {
        let region = RegionId(region_start + slice as u64);
        let instrument = InstrumentId(instrument_start + slice as u64);
        candidate.regions.insert(
            region,
            SampleRegion {
                amp_envelope: None,
                forward_loop: None,
                name: format!("{} slice {}", config.name, slice + 1),
                sample: set.sample,
                start_frame: set.markers[slice],
                end_frame: set.markers[slice + 1],
                root_note: config.root_note,
            },
        );
        candidate.instruments.insert(
            instrument,
            Instrument {
                kind: InstrumentKind::Pitched,
                name: format!("{} slice {}", config.name, slice + 1),
                sample: None,
                region: Some(region),
            },
        );
        regions.push(region);
        instruments.push(instrument);
    }
    candidate.tracks.insert(
        track,
        Track {
            name: config.name.clone(),
            note_columns: lanes.len() as u16,
        },
    );
    candidate.track_order.push(track);
    candidate.patterns.insert(
        pattern,
        Pattern {
            generation: None,
            track_aliases: Default::default(),
            name: config.name.clone(),
            rows: pattern_rows,
            tracks: BTreeMap::from([(track, rows)]),
        },
    );
    let occurrence = candidate.order.len();
    candidate.order.push(Occurrence {
        muted_tracks: Default::default(),
        section: None,
        pattern,
    });
    candidate.validate()?;
    let total_rows = candidate
        .order
        .iter()
        .try_fold(0u64, |sum, order| {
            sum.checked_add(u64::from(candidate.patterns[&order.pattern].rows))
        })
        .ok_or("Arrangement row count overflow")?;
    if total_rows as f64 * 60.0 / (candidate.bpm * f64::from(candidate.lines_per_beat)) > 600.0 {
        return Err("Compiled arrangement exceeds ten-minute render limit".into());
    }
    let mut warnings=vec!["Source beat zero is file frame 0; source BPM is your explicit assumption, not a detected tempo.".into(),"Output is ordinary editable content; later marker edits do not rewrite earlier compiled patterns.".into()];
    if !song.order.is_empty() {
        warnings.push(
            "The pattern is appended to the existing order; existing tracks/voices are retained."
                .into(),
        );
    }
    if config.target_bpm != song.bpm && !song.order.is_empty() {
        warnings.push(
            "Target BPM changes the GLOBAL song clock, including all previously ordered patterns."
                .into(),
        );
    }
    if !config.pad_tails {
        warnings.push("Tail padding is disabled; normal final SongEnd release may truncate the last native sample tails.".into());
    }
    if config.tail == TailPolicy::CutPrevious {
        warnings.push("Hard cuts target the previous generated voice and may click.".into());
    }
    if config.tail == TailPolicy::Overlap {
        warnings.push("Natural overlaps can clip; rendering rejects clipping rather than normalizing silently.".into());
    }
    let summary = CompileSummary {
        algorithm: ALGORITHM.into(),
        marker_set: config.marker_set,
        source: set.sample,
        source_sha256: source.sha256.clone(),
        source_bpm,
        previous_bpm: song.bpm,
        target_bpm: config.target_bpm,
        pattern,
        track,
        occurrence,
        regions,
        instruments,
        slice_count: slices as u16,
        note_starts: count as u32,
        note_columns: lanes.len() as u16,
        sequence_rows,
        tail_padding_rows: pattern_rows - sequence_rows,
        pattern_rows,
        warnings,
    };
    let bytes = serde_json::to_vec(&(ALGORITHM, song, config, &candidate))
        .map_err(|error| error.to_string())?;
    let id = crate::sample_asset::content_sha256(&bytes);
    Ok(CompilePreview {
        id,
        base: song.clone(),
        candidate,
        config: config.clone(),
        summary,
    })
}
