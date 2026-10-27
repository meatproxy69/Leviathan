//! Bounded document-phrase expansion. No nested generation, UI clock or DSP here.
//! The no-trigger path remains in the original evaluator for byte compatibility.
use super::{resolve_assets, Column, Command, Event, Manifest, PhrasePosition, Release, Step};
use crate::{song::*, RATE, TAIL};
use std::{collections::BTreeMap, path::Path};

const MAX_ACTIONS: usize = 65536;
const MAX_INSTANCES: usize = 256;
#[derive(Clone, Copy)]
struct Origin {
    track: TrackId,
    column: u16,
    occurrence: usize,
    row: u32,
    absolute_row: u64,
}
#[derive(Clone, Copy)]
struct Context {
    track: TrackId,
    column: u16,
    position: Option<(usize, u32)>,
    phrase: Option<PhrasePosition>,
}
impl Origin {
    fn context(self) -> Context {
        Context {
            track: self.track,
            column: self.column,
            position: Some((self.occurrence, self.row)),
            phrase: None,
        }
    }
}
#[derive(Clone, Copy)]
enum Action<'a> {
    Mute {
        track: TrackId,
    },
    Root {
        origin: Origin,
        cell: &'a Cell,
        step: Step,
        instance: Option<usize>,
    },
    Child {
        instance: usize,
        cycle: u16,
        row: u32,
        column: u16,
        cell: &'a Cell,
        step: Step,
    },
    End {
        instance: usize,
    },
}
struct Scheduled<'a> {
    key: [u64; 8],
    action: Action<'a>,
}
struct Template<'a> {
    phrase: &'a Phrase,
    id: PhraseId,
    origin: Origin,
    transpose: i16,
}
#[derive(Default)]
struct RootColumn {
    note: Column,
    instance: Option<usize>,
}
struct Instance {
    alive: bool,
    cancelled: bool,
    kit: bool,
    phrase: PhraseId,
    origin: Origin,
    instrument: InstrumentId,
    transpose: i16,
    gain: f32,
    pan: f32,
    columns: BTreeMap<u16, Column>,
    owned: Vec<(usize, u16)>,
}
fn emit(
    events: &mut Vec<Event>,
    frame: u64,
    context: Context,
    instrument: InstrumentId,
    command: Command,
) {
    events.push(Event {
        frame,
        track: context.track,
        column: context.column,
        instrument,
        position: context.position,
        command,
        phrase: context.phrase,
    });
}
fn latch(column: &mut Column, cell: &Cell) {
    if let Some(instrument) = cell.instrument {
        column.instrument = Some(instrument);
    }
    if let Some(volume) = cell.velocity {
        column.volume = volume;
    }
    if let Some(pan) = cell.pan {
        column.pan = pan;
    }
}
fn transposed(pitch: u8, transpose: i16) -> Result<u8, String> {
    let pitch = i16::from(pitch) + transpose;
    if !(0..=127).contains(&pitch) {
        Err("phrase transposition outside MIDI 0..127".into())
    } else {
        Ok(pitch as u8)
    }
}
// This applies the same scalar-column semantics as the legacy evaluator. An
// instance's gain/pan are immutable trigger snapshots, not global sampler state.
#[allow(clippy::too_many_arguments)]
fn apply_cell(
    events: &mut Vec<Event>,
    next_voice: &mut usize,
    column: &mut Column,
    cell: &Cell,
    step: Step,
    frame: u64,
    context: Context,
    gain: f32,
    pan: f32,
    transpose: i16,
    owned: Option<&mut Vec<(usize, u16)>>,
) -> Result<(), String> {
    if step == Step::Cell {
        latch(column, cell);
    }
    if step == Step::Release || step == Step::Cut {
        if let Some((instrument, voice)) = column.active.take() {
            let command = if step == Step::Cut {
                Command::Cut { voice }
            } else {
                Command::Off {
                    voice,
                    reason: Release::TimedRelease,
                }
            };
            emit(events, frame, context, instrument, command);
        }
        return Ok(());
    }
    match cell.note {
        Note::Note(pitch) => {
            let instrument = column.instrument.ok_or("note has no selected instrument")?;
            if let Some((instrument, voice)) = column.active.take() {
                emit(
                    events,
                    frame,
                    context,
                    instrument,
                    Command::Off {
                        voice,
                        reason: if step == Step::Retrigger {
                            Release::Retrigger
                        } else {
                            Release::Replaced
                        },
                    },
                );
            }
            if *next_voice > 2048 {
                return Err("Song exceeds 2048 note triggers including phrase expansion".into());
            }
            let voice = *next_voice;
            *next_voice += 1;
            emit(
                events,
                frame,
                context,
                instrument,
                Command::On {
                    voice,
                    pitch: transposed(pitch, transpose)?,
                    volume: column.volume * gain,
                    pan: (column.pan + pan).clamp(-1.0, 1.0),
                },
            );
            column.active = Some((instrument, voice));
            if let Some(owned) = owned {
                owned.push((voice, context.phrase.map_or(context.column, |p| p.column)));
            }
        }
        Note::Empty | Note::Off => {
            if let Some((instrument, voice)) = column.active {
                if let Some(value) = cell.velocity {
                    emit(
                        events,
                        frame,
                        context,
                        instrument,
                        Command::Volume {
                            voice,
                            value: value * gain,
                        },
                    );
                }
                if let Some(value) = cell.pan {
                    emit(
                        events,
                        frame,
                        context,
                        instrument,
                        Command::Pan {
                            voice,
                            value: (value + pan).clamp(-1.0, 1.0),
                        },
                    );
                }
                if cell.note == Note::Off {
                    emit(
                        events,
                        frame,
                        context,
                        instrument,
                        Command::Off {
                            voice,
                            reason: Release::ExplicitOff,
                        },
                    );
                    column.active = None;
                }
            }
        }
    }
    if cell.effects.offset.is_some() || cell.effects.direction.is_some() {
        let (instrument, voice) = column
            .active
            .ok_or("sample offset/direction requires an active voice")?;
        if matches!(cell.note, Note::Note(_)) {
            emit(
                events,
                frame,
                context,
                instrument,
                Command::SampleStart {
                    voice,
                    offset: cell.effects.offset,
                    direction: cell.effects.direction.unwrap_or(Direction::Forward),
                },
            );
        } else {
            if let Some(source_frame) = cell.effects.offset {
                emit(
                    events,
                    frame,
                    context,
                    instrument,
                    Command::Offset {
                        voice,
                        frame: source_frame,
                    },
                );
            }
            if let Some(direction) = cell.effects.direction {
                emit(
                    events,
                    frame,
                    context,
                    instrument,
                    Command::Direction { voice, direction },
                );
            }
        }
    }
    if let Some(glide) = &cell.effects.glide {
        let (instrument, voice) = column.active.ok_or("glide requires an active voice")?;
        emit(
            events,
            frame,
            context,
            instrument,
            Command::Glide {
                voice,
                target: transposed(glide.target, transpose)?,
                semitones_per_second: glide.semitones_per_second,
            },
        );
    }
    Ok(())
}
fn stop_instance(
    instances: &mut BTreeMap<usize, Instance>,
    events: &mut Vec<Event>,
    id: usize,
    frame: u64,
    position: Option<(usize, u32)>,
    cut: bool,
    reason: Release,
) {
    let Some(instance) = instances.get_mut(&id) else {
        return;
    };
    if instance.cancelled {
        return;
    }
    // A trigger-owned hard cut can occur after the phrase clock has ended:
    // released voices still belong to this instance until their tails finish.
    // Kit OneShot tails also survive natural phrase end and remain addressable
    // by a later trigger-owned timed release.
    if !instance.alive && !cut && !(instance.kit && reason == Release::TimedRelease) {
        return;
    }
    instance.alive = false;
    let context = |column| Context {
        track: instance.origin.track,
        column: instance.origin.column,
        position,
        phrase: Some(PhrasePosition {
            phrase: instance.phrase,
            instance: id,
            column,
            cycle: None,
            row: None,
        }),
    };
    if cut {
        for &(voice, column) in &instance.owned {
            emit(
                events,
                frame,
                context(column),
                instance.instrument,
                Command::Cut { voice },
            );
        }
        for column in instance.columns.values_mut() {
            column.active = None;
        }
    } else if instance.kit {
        // Kit columns can leave older pad voices ringing after another selector.
        // Owner-level phrase stops must address every owned kit voice; the pad
        // resolver then applies each pad's OneShot/Gated policy independently.
        for &(voice, column) in &instance.owned {
            emit(
                events,
                frame,
                context(column),
                instance.instrument,
                Command::Off {
                    voice,
                    reason: reason.clone(),
                },
            );
        }
        for column in instance.columns.values_mut() {
            column.active = None;
        }
    } else {
        for (&index, column) in &mut instance.columns {
            if let Some((instrument, voice)) = column.active.take() {
                emit(
                    events,
                    frame,
                    context(index),
                    instrument,
                    Command::Off {
                        voice,
                        reason: reason.clone(),
                    },
                );
            }
        }
    }
}
fn each_step(
    cell: &Cell,
    mut push: impl FnMut(f64, u8, Step) -> Result<(), String>,
) -> Result<(), String> {
    let delay = cell.delay.unwrap_or(0.0);
    push(delay, 0, Step::Cell)?;
    if let Some(retrigger) = &cell.effects.retrigger {
        for repeat in 1..=retrigger.repeats {
            push(
                delay + f64::from(repeat) * retrigger.spacing,
                repeat,
                Step::Retrigger,
            )?;
        }
    }
    let last = cell
        .effects
        .retrigger
        .as_ref()
        .map_or(delay, |r| delay + f64::from(r.repeats) * r.spacing);
    if let Some(release) = cell.effects.release {
        push(release.max(last), 17, Step::Release)?;
    }
    if let Some(cut) = cell.effects.cut {
        push(cut.max(last), 17, Step::Cut)?;
    }
    Ok(())
}
fn push_bounded<'a>(
    actions: &mut Vec<Scheduled<'a>>,
    other_count: usize,
    key: [u64; 8],
    action: Action<'a>,
) -> Result<(), String> {
    if actions.len() + other_count >= MAX_ACTIONS {
        return Err("Song exceeds 65536 expanded step commands including phrases".into());
    }
    actions.push(Scheduled { key, action });
    Ok(())
}

pub(super) fn evaluate(
    song: &Song,
    project_dir: Option<&Path>,
    check_paths: bool,
) -> Result<Manifest, String> {
    song.validate()?;
    if song.order.is_empty() || song.tracks.is_empty() {
        return Err("Song needs tracks and at least one ordered pattern".into());
    }
    let rows: u64 = song
        .order
        .iter()
        .map(|o| u64::from(song.patterns[&o.pattern].rows))
        .sum();
    let per_row = f64::from(RATE) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
    let end = (rows as f64 * per_row).round() as u64;
    if end == 0 || end > u64::from(RATE) * 600 {
        return Err("Song duration must be positive and at most ten minutes".into());
    }
    let per_beat = f64::from(RATE) * 60.0 / song.bpm;
    let song_beats = rows as f64 / f64::from(song.lines_per_beat);
    let mute_schedule = super::arrangement::MuteSchedule::new(song, RATE);
    let mut actions = Vec::new();
    let mut offset = 0u64;
    let mut populated = 0usize;
    for (occurrence, order) in song.order.iter().enumerate() {
        let pattern = &song.patterns[&order.pattern];
        for (track_index, &track) in song.track_order.iter().enumerate() {
            if order.muted_tracks.contains(&track) {
                push_bounded(
                    &mut actions,
                    0,
                    [
                        (offset as f64 * per_row).round() as u64,
                        0,
                        offset,
                        track_index as u64,
                        0,
                        0,
                        0,
                        0,
                    ],
                    Action::Mute { track },
                )?;
                continue;
            }
            if let Some(track_rows) = song.resolved_track(order.pattern, track)? {
                for (&row, columns) in track_rows {
                    for (&column, cell) in columns {
                        if *cell == Cell::default() {
                            continue;
                        }
                        populated += 1;
                        if populated > 32768 {
                            return Err("Song exceeds 32768 populated cells".into());
                        }
                        let origin = Origin {
                            track,
                            column,
                            occurrence,
                            row,
                            absolute_row: offset + u64::from(row),
                        };
                        each_step(cell, |fraction, suborder, step| {
                            let frame =
                                (((origin.absolute_row as f64 + fraction) * per_row).round()
                                    as u64)
                                    .min(end - 1);
                            if mute_schedule.contains(track, frame) {
                                return Ok(());
                            }
                            push_bounded(
                                &mut actions,
                                0,
                                [
                                    frame.min(end - 1),
                                    0,
                                    origin.absolute_row,
                                    track_index as u64,
                                    u64::from(column),
                                    u64::from(suborder),
                                    0,
                                    0,
                                ],
                                Action::Root {
                                    origin,
                                    cell,
                                    step,
                                    instance: None,
                                },
                            )
                        })?;
                    }
                }
            }
        }
        offset += u64::from(pattern.rows);
    }
    actions.sort_by_key(|action| action.key);
    let phrase_cells: BTreeMap<_, Vec<_>> = song
        .phrases
        .iter()
        .map(|(&id, p)| {
            (
                id,
                p.cells
                    .iter()
                    .flat_map(|(&row, columns)| {
                        columns
                            .iter()
                            .filter(|(_, cell)| **cell != Cell::default())
                            .map(move |(&column, cell)| (row, column, cell))
                    })
                    .collect(),
            )
        })
        .collect();
    let mut children = Vec::new();
    let mut templates = Vec::new();
    let mut trigger_instances = BTreeMap::new();
    let root_count = actions.len();
    for scheduled in &mut actions {
        let Action::Root {
            origin,
            cell,
            step: Step::Cell,
            ref mut instance,
        } = scheduled.action
        else {
            continue;
        };
        let Some(trigger) = &cell.phrase else {
            continue;
        };
        if templates.len() >= MAX_INSTANCES {
            return Err("Song exceeds 256 phrase trigger occurrences".into());
        }
        let phrase = &song.phrases[&trigger.phrase];
        let Note::Note(note) = cell.note else {
            return Err("phrase trigger requires pitched note".into());
        };
        let id = templates.len() + 1;
        *instance = Some(id);
        trigger_instances.insert((origin.absolute_row, origin.track, origin.column), id);
        templates.push(Template {
            phrase,
            id: trigger.phrase,
            origin,
            transpose: i16::from(note) - i16::from(phrase.root_note),
        });
        let trigger_frame = scheduled.key[0];
        let origin_beats = (origin.absolute_row as f64 + cell.delay.unwrap_or(0.0))
            / f64::from(song.lines_per_beat);
        let length_beats =
            f64::from(phrase.rows) * f64::from(trigger.cycles) / f64::from(phrase.lines_per_beat);
        let phrase_end = ((origin_beats + length_beats) * per_beat).round() as u64;
        let phrase_end = phrase_end.min(end).max(trigger_frame + 1);
        for cycle in 0..trigger.cycles {
            let cycle_rows = u64::from(cycle) * u64::from(phrase.rows);
            if cycle > 0
                && origin_beats + cycle_rows as f64 / f64::from(phrase.lines_per_beat) >= song_beats
            {
                break;
            }
            for &(row, column, phrase_cell) in &phrase_cells[&trigger.phrase] {
                let zero_origin = cycle == 0 && row == 0;
                let row_beats = origin_beats
                    + (cycle_rows + u64::from(row)) as f64 / f64::from(phrase.lines_per_beat);
                if row_beats >= song_beats && !zero_origin {
                    break;
                }
                each_step(phrase_cell, |fraction, suborder, step| {
                    let absolute = origin_beats
                        + ((cycle_rows + u64::from(row)) as f64 + fraction)
                            / f64::from(phrase.lines_per_beat);
                    if absolute >= song_beats && !(zero_origin && fraction == 0.0) {
                        return Ok(());
                    }
                    let frame = ((absolute * per_beat).round() as u64)
                        .min(phrase_end - 1)
                        .max(trigger_frame);
                    push_bounded(
                        &mut children,
                        root_count,
                        [
                            frame,
                            2,
                            id as u64,
                            u64::from(cycle),
                            u64::from(row),
                            u64::from(column),
                            u64::from(suborder),
                            0,
                        ],
                        Action::Child {
                            instance: id,
                            cycle,
                            row,
                            column,
                            cell: phrase_cell,
                            step,
                        },
                    )
                })?;
            }
        }
        push_bounded(
            &mut children,
            root_count,
            [phrase_end, 1, id as u64, 0, 0, 0, 0, 0],
            Action::End { instance: id },
        )?;
    }
    // Timed stops written on a trigger own that specific instance, even after
    // its natural end. They must not target whichever instance/ordinary note
    // happens to occupy the parent column when the command executes.
    for scheduled in &mut actions {
        if let Action::Root {
            origin,
            cell,
            ref mut instance,
            ..
        } = scheduled.action
        {
            if cell.phrase.is_some() {
                *instance = trigger_instances
                    .get(&(origin.absolute_row, origin.track, origin.column))
                    .copied();
            }
        }
    }
    actions.extend(children);
    actions.sort_by_key(|action| action.key);
    let mut roots: BTreeMap<(TrackId, u16), RootColumn> = BTreeMap::new();
    let mut instances: BTreeMap<usize, Instance> = BTreeMap::new();
    let mut events = Vec::new();
    let mut next_voice = 1usize;
    for scheduled in actions {
        let frame = scheduled.key[0];
        match scheduled.action {
            Action::Mute { track } => {
                for ((owner, _), root) in &mut roots {
                    if *owner == track {
                        root.note.active = None;
                        root.instance = None;
                    }
                }
                for instance in instances.values_mut().filter(|i| i.origin.track == track) {
                    instance.alive = false;
                    instance.cancelled = true;
                    for column in instance.columns.values_mut() {
                        column.active = None;
                    }
                }
                // Addressed releases are emitted after kit policies resolve,
                // including older OneShots no longer in a column pointer.
            }
            Action::Root {
                origin,
                cell,
                step,
                instance,
            } => {
                let root = roots.entry((origin.track, origin.column)).or_default();
                let context = origin.context();
                if step == Step::Cell {
                    latch(&mut root.note, cell);
                }
                if step == Step::Release || step == Step::Cut {
                    if let Some(id) = instance.or(root.instance) {
                        if root.instance == Some(id) {
                            root.instance = None;
                        }
                        stop_instance(
                            &mut instances,
                            &mut events,
                            id,
                            frame,
                            context.position,
                            step == Step::Cut,
                            Release::TimedRelease,
                        );
                    } else {
                        apply_cell(
                            &mut events,
                            &mut next_voice,
                            &mut root.note,
                            cell,
                            step,
                            frame,
                            context,
                            1.0,
                            0.0,
                            0,
                            None,
                        )?;
                    }
                    continue;
                }
                if cell.phrase.is_some() {
                    if let Some((instrument, voice)) = root.note.active.take() {
                        emit(
                            &mut events,
                            frame,
                            context,
                            instrument,
                            Command::Off {
                                voice,
                                reason: Release::Replaced,
                            },
                        );
                    }
                    if let Some(old) = root.instance.take() {
                        stop_instance(
                            &mut instances,
                            &mut events,
                            old,
                            frame,
                            context.position,
                            false,
                            Release::Replaced,
                        );
                    }
                    let id = instance.ok_or("missing compiled phrase instance")?;
                    let template = &templates[id - 1];
                    root.note.instrument = Some(template.phrase.instrument);
                    root.instance = Some(id);
                    instances.insert(
                        id,
                        Instance {
                            alive: true,
                            cancelled: false,
                            kit: matches!(
                                song.instruments[&template.phrase.instrument].kind,
                                InstrumentKind::DrumKit { .. }
                            ),
                            phrase: template.id,
                            origin: template.origin,
                            instrument: template.phrase.instrument,
                            transpose: template.transpose,
                            gain: root.note.volume,
                            pan: root.note.pan,
                            columns: BTreeMap::new(),
                            owned: Vec::new(),
                        },
                    );
                    continue;
                }
                if let Some(id) = root.instance {
                    match cell.note {
                        Note::Note(_) => {
                            stop_instance(
                                &mut instances,
                                &mut events,
                                id,
                                frame,
                                context.position,
                                false,
                                Release::Replaced,
                            );
                            root.instance = None;
                        }
                        Note::Off => {
                            stop_instance(
                                &mut instances,
                                &mut events,
                                id,
                                frame,
                                context.position,
                                false,
                                Release::ExplicitOff,
                            );
                            root.instance = None;
                            continue;
                        }
                        Note::Empty => {
                            if cell.effects.glide.is_some()
                                || cell.effects.offset.is_some()
                                || cell.effects.direction.is_some()
                            {
                                return Err("parent per-voice controls cannot address a running phrase instance; edit phrase cells or stop it".into());
                            }
                            continue;
                        }
                    }
                }
                apply_cell(
                    &mut events,
                    &mut next_voice,
                    &mut root.note,
                    cell,
                    step,
                    frame,
                    context,
                    1.0,
                    0.0,
                    0,
                    None,
                )?;
            }
            Action::Child {
                instance: id,
                cycle,
                row,
                column,
                cell,
                step,
            } => {
                let Some(instance) = instances.get_mut(&id).filter(|i| i.alive) else {
                    continue;
                };
                let mut context = instance.origin.context();
                context.phrase = Some(PhrasePosition {
                    phrase: instance.phrase,
                    instance: id,
                    column,
                    cycle: Some(cycle),
                    row: Some(row),
                });
                let state = instance.columns.entry(column).or_insert_with(|| Column {
                    instrument: Some(instance.instrument),
                    ..Default::default()
                });
                apply_cell(
                    &mut events,
                    &mut next_voice,
                    state,
                    cell,
                    step,
                    frame,
                    context,
                    instance.gain,
                    instance.pan,
                    instance.transpose,
                    Some(&mut instance.owned),
                )?;
            }
            Action::End { instance: id } => {
                let template = &templates[id - 1];
                stop_instance(
                    &mut instances,
                    &mut events,
                    id,
                    frame,
                    if frame == end {
                        None
                    } else {
                        Some((template.origin.occurrence, template.origin.row))
                    },
                    false,
                    if frame == end {
                        Release::SongEnd
                    } else {
                        Release::PhraseEnd
                    },
                );
                if let Some(root) = roots.get_mut(&(template.origin.track, template.origin.column))
                {
                    if root.instance == Some(id) {
                        root.instance = None;
                    }
                }
            }
        }
    }
    if next_voice == 1 && !super::arrangement::permits_silence(song)? {
        return Err("Song contains no note triggers".into());
    }
    for ((track, column), root) in roots {
        if let Some((instrument, voice)) = root.note.active {
            emit(
                &mut events,
                end,
                Context {
                    track,
                    column,
                    position: None,
                    phrase: None,
                },
                instrument,
                Command::Off {
                    voice,
                    reason: Release::SongEnd,
                },
            );
        }
    }
    // Phrase end actions may have reached song end before root-note final offs.
    // Stable sorting preserves all established within-frame ownership order.
    events.sort_by_key(|event| event.frame);
    let assets = resolve_assets(song, &events, project_dir, check_paths)?;
    Ok(Manifest {
        schema: 4,
        evaluator: "leviathan-song-v4".into(),
        sample_rate: RATE,
        bpm: song.bpm,
        lines_per_beat: song.lines_per_beat,
        musical_frames: end,
        tail_frames: TAIL,
        master_gain: 0.5,
        assets,
        events,
        regions: BTreeMap::new(),
        pad_voices: BTreeMap::new(),
        multisample_voices: BTreeMap::new(),
        arrangement: None,
        track_mix: BTreeMap::new(),
    })
}
