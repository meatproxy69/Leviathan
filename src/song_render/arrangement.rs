//! Occurrence mute semantics after pad policy resolution, using the same events.
//! No DSP gate: held voices release; already releasing tails remain untouched.
use super::*;
const MAX: usize = 65536;
pub(super) type MuteSchedule = crate::song::FrameMutes;
fn push(events: &mut Vec<Event>, event: Event) -> Result<(), String> {
    if events.len() >= MAX {
        return Err("Song exceeds 65536 events including track mutes".into());
    }
    events.push(event);
    Ok(())
}
/// Silence is valid when a real ordered trigger is explicitly suppressed by
/// an occurrence mute; this does not make an ordinary blank Song renderable.
pub(super) fn permits_silence(song: &Song) -> Result<bool, String> {
    if song.order.iter().all(|o| o.muted_tracks.is_empty()) {
        return Ok(false);
    }
    let schedule = MuteSchedule::new(song, RATE);
    let per_row = f64::from(RATE) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
    let end = (song
        .order
        .iter()
        .map(|o| u64::from(song.patterns[&o.pattern].rows))
        .sum::<u64>() as f64
        * per_row)
        .round() as u64;
    let mut origin = 0u64;
    let mut inspected = 0usize;
    for occurrence in &song.order {
        for &track in &song.track_order {
            if let Some(rows) = song.resolved_track(occurrence.pattern, track)? {
                for (&row, columns) in rows {
                    for cell in columns.values() {
                        inspected += 1;
                        if inspected > 32768 {
                            return Err(
                                "Song exceeds 32768 cells while checking muted silence".into()
                            );
                        }
                        let frame = (((origin + u64::from(row)) as f64 + cell.delay.unwrap_or(0.0))
                            * per_row)
                            .round() as u64;
                        if matches!(cell.note, Note::Note(_))
                            && schedule.contains(track, frame.min(end.saturating_sub(1)))
                        {
                            return Ok(true);
                        }
                    }
                }
            }
        }
        origin += u64::from(song.patterns[&occurrence.pattern].rows);
    }
    Ok(false)
}
pub(super) fn resolve(song: &Song, manifest: &mut Manifest) -> Result<(), String> {
    let per_row = f64::from(RATE) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
    let mut aliases = Vec::new();
    let mut mutes = Vec::new();
    let mut row = 0u64;
    for (occurrence, order) in song.order.iter().enumerate() {
        let pattern = &song.patterns[&order.pattern];
        let start_frame = (row as f64 * per_row).round() as u64;
        row += u64::from(pattern.rows);
        let end_frame = (row as f64 * per_row).round() as u64;
        for &track in pattern.track_aliases.keys() {
            if aliases.len() >= MAX {
                return Err("Song exceeds 65536 ordered alias blocks".into());
            }
            aliases.push(RenderAlias {
                occurrence,
                track,
                chain: song.resolve_track_chain(order.pattern, track)?,
            });
        }
        for &track in &order.muted_tracks {
            mutes.push(TrackMute {
                occurrence,
                track,
                start_frame,
                end_frame,
            });
        }
    }
    if aliases.is_empty() && mutes.is_empty() {
        return Ok(());
    }
    if !mutes.is_empty() {
        // Kit endings are appended by its resolver. A stable sort retains all
        // existing same-frame event ordering while merging explicit boundaries.
        manifest.events.sort_by_key(|event| event.frame);
        let mut events = Vec::with_capacity(manifest.events.len());
        let mut held: BTreeMap<usize, Event> = BTreeMap::new();
        let mut cancelled = BTreeSet::new();
        let mut muted_until = BTreeMap::new();
        let mut boundary = 0;
        for event in std::mem::take(&mut manifest.events) {
            while boundary < mutes.len() && mutes[boundary].start_frame <= event.frame {
                let mute = &mutes[boundary];
                muted_until.insert(mute.track, mute.end_frame);
                let voices: Vec<_> = held
                    .iter()
                    .filter_map(|(&voice, origin)| (origin.track == mute.track).then_some(voice))
                    .collect();
                for voice in voices {
                    let mut stop = held.remove(&voice).expect("collected held voice");
                    stop.frame = mute.start_frame;
                    stop.position = Some((mute.occurrence, 0));
                    if let Some(phrase) = &mut stop.phrase {
                        phrase.cycle = None;
                        phrase.row = None;
                    }
                    stop.command = Command::Off {
                        voice,
                        reason: Release::TrackMute,
                    };
                    push(&mut events, stop)?;
                    cancelled.insert(voice);
                }
                boundary += 1;
            }
            let voice = event.command.voice();
            if cancelled.contains(&voice) {
                continue;
            }
            if muted_until
                .get(&event.track)
                .is_some_and(|end| event.frame < *end)
            {
                // Includes delayed prior-row events rounded onto a mute start.
                // Their future controls must not revive a never-started voice.
                if matches!(event.command, Command::On { .. }) {
                    cancelled.insert(voice);
                }
                continue;
            }
            match event.command {
                Command::On { .. } => {
                    held.insert(voice, event.clone());
                }
                Command::Off { .. } | Command::Cut { .. } => {
                    held.remove(&voice);
                }
                _ => (),
            }
            push(&mut events, event)?;
        }
        // There is normally a SongEnd event after every mute. Still handle
        // boundaries after the final explicit event for fully finite sources.
        while boundary < mutes.len() {
            let mute = &mutes[boundary];
            let voices: Vec<_> = held
                .iter()
                .filter_map(|(&voice, origin)| (origin.track == mute.track).then_some(voice))
                .collect();
            for voice in voices {
                let mut stop = held.remove(&voice).expect("collected held voice");
                stop.frame = mute.start_frame;
                stop.position = Some((mute.occurrence, 0));
                if let Some(phrase) = &mut stop.phrase {
                    phrase.cycle = None;
                    phrase.row = None;
                }
                stop.command = Command::Off {
                    voice,
                    reason: Release::TrackMute,
                };
                push(&mut events, stop)?;
            }
            boundary += 1;
        }
        if !events
            .iter()
            .any(|event| matches!(event.command, Command::On { .. }))
            && !permits_silence(song)?
        {
            return Err("Song contains no note triggers after occurrence mutes".into());
        }
        manifest.events = events;
    }
    manifest.arrangement = Some(RenderArrangement { aliases, mutes });
    manifest.schema = 7;
    manifest.evaluator = "leviathan-song-v7".into();
    Ok(())
}
