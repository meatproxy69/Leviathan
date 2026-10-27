//! Bounded one-shot phrase compilation. The flat evaluator remains the sole note/FX interpreter.
use crate::{
    song::*,
    song_render::{self, Command, Event, Manifest, PhraseOrigin, Release},
    RATE,
};
use std::{collections::BTreeMap, path::Path};
const PHRASE_COLUMNS: u16 = 16;
fn virtual_column(parent: u16, local: u16) -> u16 {
    256 + parent * PHRASE_COLUMNS + local
}
type Source = (usize, u32, TrackId, u16);
struct Parent<'a> {
    frame: u64,
    origin: f64,
    absolute_row: u64,
    track_index: usize,
    column: u16,
    cut: bool,
    occurrence: usize,
    row: u32,
    track: TrackId,
    cell: &'a Cell,
}
impl Parent<'_> {
    fn source(&self) -> Source {
        (self.occurrence, self.row, self.track, self.column)
    }
}
struct Group {
    owner: InstrumentId,
    phrase: PhraseId,
    origin: f64,
    start: u64,
    end: u64,
    hard: bool,
    reason: Release,
    index: usize,
    source: Source,
    transpose: i16,
    volume: f32,
    pan: f32,
}
fn voice(command: &Command) -> Option<usize> {
    match *command {
        Command::On { voice, .. }
        | Command::Off { voice, .. }
        | Command::Volume { voice, .. }
        | Command::Pan { voice, .. }
        | Command::Speed { voice, .. } => Some(voice),
        Command::Cut => None,
    }
}
fn remap(command: &mut Command, id: usize) {
    match command {
        Command::On { voice, .. }
        | Command::Off { voice, .. }
        | Command::Volume { voice, .. }
        | Command::Pan { voice, .. }
        | Command::Speed { voice, .. } => *voice = id,
        Command::Cut => (),
    }
}
pub(crate) fn evaluate(
    song: &Song,
    base: Option<&Path>,
    allow_empty: bool,
) -> Result<Manifest, String> {
    song.validate()?;
    let mut ordinary = song.clone();
    for pattern in ordinary.patterns.values_mut() {
        for rows in pattern.tracks.values_mut() {
            for columns in rows.values_mut() {
                for cell in columns.values_mut() {
                    if cell.phrase.take().is_some() {
                        cell.note = Note::Off;
                        cell.effects = StepEffects::default();
                    }
                }
            }
        }
    }
    let mut manifest = song_render::evaluate_flat(&ordinary, base, true, 0.0)?;
    let per_row = RATE as f64 * 60.0 / (song.bpm * song.lines_per_beat as f64);
    let song_end = manifest.musical_frames;
    let mut parents = Vec::new();
    let mut offset = 0u64;
    let mut populated = 0usize;
    for (occurrence, order) in song.order.iter().enumerate() {
        let pattern = &song.patterns[&order.pattern];
        for (track_index, track) in song.track_order.iter().enumerate() {
            if let Some(rows) = pattern.tracks.get(track) {
                for (row, columns) in rows {
                    for (column, cell) in columns {
                        if *cell == Cell::default() {
                            continue;
                        }
                        populated += 1;
                        if populated > 32768 {
                            return Err("Song exceeds 32768 populated cells".into());
                        }
                        let absolute_row = offset + *row as u64;
                        let delay = cell.delay.unwrap_or(0.0);
                        let origin =
                            ((absolute_row as f64 + delay) * per_row).min((song_end - 1) as f64);
                        let frame = (origin.round() as u64).min(song_end - 1);
                        parents.push(Parent {
                            frame,
                            origin,
                            absolute_row,
                            track_index,
                            column: *column,
                            cut: false,
                            occurrence,
                            row: *row,
                            track: *track,
                            cell,
                        });
                        if let Some(cut) = cell.effects.cut {
                            let at = if cut == 0.0 {
                                frame
                            } else {
                                (((absolute_row as f64 + delay + (1.0 - delay) * cut) * per_row)
                                    .round() as u64)
                                    .max(frame)
                            };
                            parents.push(Parent {
                                frame: at.min(song_end),
                                origin,
                                absolute_row,
                                track_index,
                                column: *column,
                                cut: true,
                                occurrence,
                                row: *row,
                                track: *track,
                                cell,
                            });
                        }
                    }
                }
            }
        }
        offset += pattern.rows as u64;
    }
    parents.sort_by_key(|p| (p.frame, p.absolute_row, p.track_index, p.column, p.cut));
    let mut cell_index = BTreeMap::new();
    let mut cut_index = BTreeMap::new();
    let mut memory: BTreeMap<(TrackId, u16), (Option<InstrumentId>, f32, f32)> = BTreeMap::new();
    let mut current: BTreeMap<(TrackId, u16), usize> = BTreeMap::new();
    let mut last_owner = BTreeMap::new();
    let mut groups: Vec<Group> = Vec::new();
    let mut cuts = Vec::new();
    for (index, parent) in parents.iter().enumerate() {
        let key = (parent.track, parent.column);
        if parent.cut {
            cut_index.insert(parent.source(), index);
            if let Some(group) = current.remove(&key).map(|i| &mut groups[i]) {
                if parent.frame <= group.end {
                    group.end = parent.frame;
                    group.hard = true;
                }
            }
            if let Some(owner) = last_owner.get(&key) {
                cuts.push((parent, index, *owner));
            }
            continue;
        }
        cell_index.insert(parent.source(), index);
        let state = memory.entry(key).or_insert((None, 1.0, 0.0));
        if let Some(instrument) = parent.cell.instrument {
            state.0 = Some(instrument);
        }
        if let Some(volume) = parent.cell.velocity {
            state.1 = volume;
        }
        if let Some(pan) = parent.cell.pan {
            state.2 = pan;
        }
        if parent.cell.note != Note::Empty {
            if let Some(group) = current.remove(&key).map(|i| &mut groups[i]) {
                if parent.frame <= group.end {
                    group.end = parent.frame;
                    group.reason = if parent.cell.note == Note::Off {
                        Release::ExplicitOff
                    } else {
                        Release::Replaced
                    };
                }
            }
        }
        let Some(phrase_id) = parent.cell.phrase else {
            continue;
        };
        let owner = state.0.ok_or("Phrase trigger has no selected instrument")?;
        let phrase = song.instruments[&owner]
            .phrases
            .get(&phrase_id)
            .ok_or_else(|| format!("Instrument {} has no phrase {}", owner.0, phrase_id.0))?;
        let Note::Note(pitch) = parent.cell.note else {
            return Err("Phrase trigger requires a note".into());
        };
        if groups.len() >= 256 {
            return Err("Song exceeds 256 phrase launches".into());
        }
        let natural = (parent.origin
            + phrase.rows as f64 * RATE as f64 * 60.0 / (song.bpm * phrase.lines_per_beat as f64))
            .round() as u64;
        current.insert(key, groups.len());
        last_owner.insert(key, owner);
        groups.push(Group {
            owner,
            phrase: phrase_id,
            origin: parent.origin,
            start: parent.frame,
            end: natural.min(song_end),
            hard: false,
            reason: if natural >= song_end {
                Release::SongEnd
            } else {
                Release::PhraseEnd
            },
            index,
            source: parent.source(),
            transpose: pitch as i16 - phrase.root_note as i16,
            volume: state.1,
            pan: state.2,
        });
    }
    // Stable source order preserves zero-time attacks/cuts and makes row-end gates precede new launches.
    let mut tagged = Vec::new();
    let mut next_voice = 1usize;
    for (sequence, mut event) in std::mem::take(&mut manifest.events).into_iter().enumerate() {
        if let Command::On { voice, .. } = event.command {
            next_voice = next_voice.max(voice + 1);
        }
        let index = event
            .position
            .map(|(occ, row)| {
                let key = (occ, row, event.track, event.column);
                if matches!(event.command, Command::Cut) {
                    *cut_index.get(&key).unwrap_or(&usize::MAX)
                } else {
                    *cell_index.get(&key).unwrap_or(&usize::MAX)
                }
            })
            .unwrap_or(usize::MAX);
        if index != usize::MAX && parents[index].cell.phrase.is_some() {
            if let Command::Off { reason, .. } = &mut event.command {
                *reason = Release::Replaced;
            }
        }
        tagged.push((event.frame, index, 0u8, sequence, event));
    }
    for group in &groups {
        let phrase = &song.instruments[&group.owner].phrases[&group.phrase];
        let mut mini = Song {
            generation_recipes: BTreeMap::new(),
            schema: 1,
            title: phrase.name.clone(),
            bpm: song.bpm,
            lines_per_beat: phrase.lines_per_beat,
            samples: song.samples.clone(),
            regions: song.regions.clone(),
            instruments: BTreeMap::from([(
                group.owner,
                Instrument {
                    name: song.instruments[&group.owner].name.clone(),
                    sample: song.instruments[&group.owner].sample.clone(),
                    region: song.instruments[&group.owner].region,
                    phrases: BTreeMap::new(),
                },
            )]),
            tracks: BTreeMap::new(),
            track_order: Vec::new(),
            patterns: BTreeMap::new(),
            order: Vec::new(),
        };
        mini.tracks = BTreeMap::from([(
            TrackId(1),
            Track {
                name: phrase.name.clone(),
                note_columns: phrase.note_columns,
            },
        )]);
        mini.track_order = vec![TrackId(1)];
        mini.order = vec![Occurrence::new(PatternId(1))];
        let mut cells = phrase.cells.clone();
        for columns in cells.values_mut() {
            for cell in columns.values_mut() {
                if let Note::Note(pitch) = cell.note {
                    let pitch = pitch as i16 + group.transpose;
                    if !(0..=127).contains(&pitch) {
                        return Err(format!(
                            "Instrument {}, phrase {} transposes outside MIDI range",
                            group.owner.0, group.phrase.0
                        ));
                    }
                    cell.note = Note::Note(pitch as u8);
                }
                cell.instrument = Some(group.owner);
                if let Some(volume) = &mut cell.velocity {
                    *volume *= group.volume;
                }
                if let Some(pan) = &mut cell.pan {
                    *pan = (*pan + group.pan).clamp(-1.0, 1.0);
                }
            }
        }
        for column in 0..phrase.note_columns {
            let cell = cells.entry(0).or_default().entry(column).or_default();
            cell.instrument = Some(group.owner);
            cell.velocity.get_or_insert(group.volume);
            cell.pan.get_or_insert(group.pan);
        }
        mini.patterns = BTreeMap::from([(
            PatternId(1),
            Pattern {
                name: phrase.name.clone(),
                rows: phrase.rows,
                tracks: BTreeMap::from([(TrackId(1), cells)]),
                aliases: BTreeMap::new(),
            },
        )]);
        let inner = song_render::evaluate_flat(&mini, base, true, group.origin)?;
        manifest.assets.extend(inner.assets);
        let mut ids = BTreeMap::new();
        let mut active = BTreeMap::new();
        for (sequence, mut event) in inner.events.into_iter().enumerate() {
            let natural_terminal = event.frame == inner.musical_frames && event.frame == group.end;
            if event.frame > group.end
                || (event.frame == group.end && group.end != group.start && !natural_terminal)
            {
                continue;
            }
            if let Command::On { voice, .. } = event.command {
                if next_voice > 2048 {
                    return Err("Song exceeds 2048 expanded note triggers".into());
                }
                ids.insert(voice, next_voice);
                next_voice += 1;
            }
            if let Some(old) = voice(&event.command) {
                let Some(&new) = ids.get(&old) else {
                    continue;
                };
                remap(&mut event.command, new);
            }
            if event.position.is_none() {
                if let Command::Off { reason, .. } = &mut event.command {
                    *reason = group.reason.clone();
                }
            }
            match event.command {
                Command::On { voice, .. } => {
                    active.insert(event.column, voice);
                }
                Command::Off { voice, .. } => {
                    if active.get(&event.column) == Some(&voice) {
                        active.remove(&event.column);
                    }
                }
                Command::Cut => {
                    active.remove(&event.column);
                }
                _ => (),
            }
            event.phrase = Some(PhraseOrigin {
                instrument: group.owner,
                phrase: group.phrase,
                trigger_column: group.source.3,
                row: event.position.map(|(_, row)| row),
            });
            event.position = Some((group.source.0, group.source.1));
            event.track = group.source.2;
            event.column = virtual_column(group.source.3, event.column);
            tagged.push((event.frame, group.index, 1, sequence, event));
        }
        if !group.hard {
            for (sequence, (column, voice)) in active.into_iter().enumerate() {
                let event = Event {
                    frame: group.end,
                    track: group.source.2,
                    column: virtual_column(group.source.3, column),
                    instrument: group.owner,
                    position: Some((group.source.0, group.source.1)),
                    command: Command::Off {
                        voice,
                        reason: group.reason.clone(),
                    },
                    phrase: Some(PhraseOrigin {
                        instrument: group.owner,
                        phrase: group.phrase,
                        trigger_column: group.source.3,
                        row: None,
                    }),
                };
                tagged.push((event.frame, group.index, 2, sequence, event));
            }
        }
        if tagged.len() > 131072 {
            return Err("Song exceeds 131072 expanded phrase events".into());
        }
    }
    for (parent, index, owner) in cuts {
        // Main-column cut may already be present for an ordinary note. Only phrase triggers need an extra one.
        if parent.cell.phrase.is_some() {
            let event = Event {
                frame: parent.frame,
                track: parent.track,
                column: parent.column,
                instrument: owner,
                position: Some((parent.occurrence, parent.row)),
                command: Command::Cut,
                phrase: None,
            };
            tagged.push((event.frame, index, 2, 0, event));
        }
        for column in 0..PHRASE_COLUMNS {
            let event = Event {
                frame: parent.frame,
                track: parent.track,
                column: virtual_column(parent.column, column),
                instrument: owner,
                position: Some((parent.occurrence, parent.row)),
                command: Command::Cut,
                phrase: None,
            };
            tagged.push((event.frame, index, 2, column as usize + 1, event));
        }
    }
    if next_voice == 1 && !allow_empty {
        return Err("Song contains no note triggers".into());
    }
    if tagged.len() > 131072 {
        return Err("Song exceeds 131072 expanded phrase events".into());
    }
    tagged.sort_by_key(|event| (event.0, event.1, event.2, event.3));
    manifest.events = tagged
        .into_iter()
        .map(|(_, _, _, _, event)| event)
        .collect();
    manifest.schema = 3;
    manifest.evaluator = "leviathan-song-v3".into();
    Ok(manifest)
}
