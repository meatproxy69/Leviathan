//! Bounded track-block content sharing; aliases never share playback state.
use super::*;
impl Song {
    /// Swap complete occurrence records (including mutes), matching order arrows.
    pub fn move_occurrence(&mut self, from: usize, to: usize) -> Result<(), String> {
        if from >= self.order.len() || to >= self.order.len() {
            return Err("occurrence move outside order".into());
        }
        let mut candidate = self.clone();
        candidate.order.swap(from, to);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }

    /// Resolve at most 32 alias edges. Row count and column geometry must match
    /// every edge exactly. Empty terminal tracks are legitimate shared content.
    pub fn resolve_track_source(
        &self,
        pattern: PatternId,
        track: TrackId,
    ) -> Result<(PatternId, TrackId), String> {
        let chain = self.resolve_track_chain(pattern, track)?;
        let source = chain.last().expect("nonempty alias chain");
        Ok((source.pattern, source.track))
    }
    /// Target first, terminal last; at most 33 entries, no recursive lookup.
    pub fn resolve_track_chain(
        &self,
        pattern: PatternId,
        track: TrackId,
    ) -> Result<Vec<TrackAlias>, String> {
        let mut key = TrackAlias { pattern, track };
        let mut chain = Vec::new();
        let mut seen = BTreeSet::new();
        for depth in 0..=32 {
            chain.push(key);
            let fail = |message: &str| format!("{message}: {chain:?}");
            if !seen.insert((key.pattern, key.track)) {
                return Err(fail("track alias cycle"));
            }
            let pattern = self
                .patterns
                .get(&key.pattern)
                .ok_or_else(|| fail("unknown alias pattern"))?;
            let track = self
                .tracks
                .get(&key.track)
                .ok_or_else(|| fail("unknown alias track"))?;
            let Some(alias) = pattern.track_aliases.get(&key.track) else {
                return Ok(chain);
            };
            if depth == 32 {
                return Err(fail("track alias exceeds 32 links"));
            }
            if pattern.tracks.contains_key(&key.track) {
                return Err(fail(
                    "aliased track cannot also contain local rows; make unique first",
                ));
            }
            let source = self
                .patterns
                .get(&alias.pattern)
                .ok_or_else(|| format!("{} -> {alias:?}", fail("unknown alias source pattern")))?;
            let source_track = self
                .tracks
                .get(&alias.track)
                .ok_or_else(|| format!("{} -> {alias:?}", fail("unknown alias source track")))?;
            if source.rows != pattern.rows || source_track.note_columns != track.note_columns {
                return Err(format!(
                    "{} -> {alias:?}",
                    fail("track alias requires identical source/target row and note-column counts")
                ));
            }
            key = *alias;
        }
        unreachable!()
    }
    pub fn resolved_track(
        &self,
        pattern: PatternId,
        track: TrackId,
    ) -> Result<Option<&TrackPattern>, String> {
        let (pattern, track) = self.resolve_track_source(pattern, track)?;
        Ok(self.patterns[&pattern].tracks.get(&track))
    }
    /// Normal target edits never silently change shared source content.
    pub fn require_local_track(&self, pattern: PatternId, track: TrackId) -> Result<(), String> {
        let pattern = self.patterns.get(&pattern).ok_or("unknown pattern")?;
        if !self.tracks.contains_key(&track) {
            return Err("unknown track".into());
        }
        if pattern.track_aliases.contains_key(&track) {
            Err("Aliased track is read-only: Make unique or explicitly edit its source".into())
        } else {
            Ok(())
        }
    }
    pub(super) fn validate_arrangement(&self) -> Result<(), String> {
        let mut aliases = 0usize;
        let mut expanded_cells = 0usize;
        let mut expanded_rows = 0usize;
        let mut counts = BTreeMap::new();
        let mut pattern_counts = BTreeMap::new();
        for (&id, pattern) in &self.patterns {
            let mut pattern_rows = 0usize;
            let mut pattern_cells = 0usize;
            aliases = aliases.saturating_add(pattern.track_aliases.len());
            if aliases > 4096 {
                return Err("song exceeds 4096 track aliases".into());
            }
            for &track in pattern.track_aliases.keys() {
                let source = self.resolve_track_source(id, track)?;
                let (rows, cells) = *counts.entry(source).or_insert_with(|| {
                    self.patterns[&source.0]
                        .tracks
                        .get(&source.1)
                        .map_or((0, 0), |rows| {
                            (rows.len(), rows.values().map(BTreeMap::len).sum::<usize>())
                        })
                });
                pattern_rows = pattern_rows.saturating_add(rows);
                pattern_cells = pattern_cells.saturating_add(cells);
                expanded_cells = expanded_cells.saturating_add(cells);
                expanded_rows = expanded_rows.saturating_add(rows);
                if expanded_cells > 65536 {
                    return Err("track aliases exceed 65536 resolved cells".into());
                }
                if expanded_rows > 65536 {
                    return Err("track aliases exceed 65536 resolved row containers".into());
                }
            }
            pattern_counts.insert(id, (pattern_rows, pattern_cells));
        }
        let mut mutes = 0usize;
        let mut ordered_rows = 0usize;
        let mut ordered_cells = 0usize;
        let mut ordered_aliases = 0usize;
        let mut section_ids = BTreeSet::new();
        for occurrence in &self.order {
            let (rows, cells) = pattern_counts[&occurrence.pattern];
            ordered_rows = ordered_rows.saturating_add(rows);
            ordered_cells = ordered_cells.saturating_add(cells);
            if ordered_rows > 65536 || ordered_cells > 65536 {
                return Err("song exceeds 65536 ordered alias cells or row containers".into());
            }
            ordered_aliases = ordered_aliases
                .saturating_add(self.patterns[&occurrence.pattern].track_aliases.len());
            if ordered_aliases > 65536 {
                return Err("song exceeds 65536 ordered alias blocks".into());
            }
            mutes = mutes.saturating_add(occurrence.muted_tracks.len());
            if mutes > 65536 {
                return Err("song exceeds 65536 occurrence track mutes".into());
            }
            if occurrence
                .muted_tracks
                .iter()
                .any(|id| !self.tracks.contains_key(id))
            {
                return Err("occurrence mute references unknown track".into());
            }
            if let Some(section) = &occurrence.section {
                if section.id.0 == 0
                    || !section_ids.insert(section.id)
                    || section.name.trim().is_empty()
                    || section.name.chars().count() > 64
                {
                    return Err(
                        "Sections need unique nonzero IDs and names of 1..64 characters".into(),
                    );
                }
            }
        }
        Ok(())
    }
    /// Some links an empty local block; None explicitly unlinks to an empty block.
    pub fn set_track_alias(
        &mut self,
        pattern: PatternId,
        track: TrackId,
        source: Option<TrackAlias>,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let target = candidate
            .patterns
            .get_mut(&pattern)
            .ok_or("unknown pattern")?;
        if let Some(source) = source {
            if target
                .tracks
                .get(&track)
                .is_some_and(|rows| !rows.is_empty())
            {
                return Err(
                    "cannot alias over local track content; clear it explicitly first".into(),
                );
            }
            target.tracks.remove(&track);
            target.track_aliases.insert(track, source);
        } else {
            target.track_aliases.remove(&track);
        }
        if !candidate.tracks.contains_key(&track) {
            return Err("unknown track".into());
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Materialize the exact terminal block, leaving the source and other users shared.
    pub fn make_track_unique(&mut self, pattern: PatternId, track: TrackId) -> Result<(), String> {
        self.validate()?;
        let content = self.resolved_track(pattern, track)?.cloned();
        let mut candidate = self.clone();
        let target = candidate
            .patterns
            .get_mut(&pattern)
            .ok_or("unknown pattern")?;
        if target.track_aliases.remove(&track).is_none() {
            return Ok(());
        }
        if let Some(content) = content {
            target.tracks.insert(track, content);
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn set_occurrence_track_mute(
        &mut self,
        occurrence: usize,
        track: TrackId,
        muted: bool,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        if !candidate.tracks.contains_key(&track) {
            return Err("unknown mute track".into());
        }
        let target = candidate
            .order
            .get_mut(occurrence)
            .ok_or("unknown occurrence")?;
        if muted {
            target.muted_tracks.insert(track);
        } else {
            target.muted_tracks.remove(&track);
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
}

pub(crate) struct FrameMutes(BTreeMap<TrackId, Vec<(u64, u64)>>);
impl FrameMutes {
    pub(crate) fn new(song: &Song, rate: u32) -> Self {
        let per_row = f64::from(rate) * 60.0 / (song.bpm * f64::from(song.lines_per_beat));
        let mut ranges: BTreeMap<TrackId, Vec<(u64, u64)>> = BTreeMap::new();
        let mut row = 0u64;
        for occurrence in &song.order {
            let start = (row as f64 * per_row).round() as u64;
            row += u64::from(song.patterns[&occurrence.pattern].rows);
            let end = (row as f64 * per_row).round() as u64;
            for &track in &occurrence.muted_tracks {
                ranges.entry(track).or_default().push((start, end));
            }
        }
        Self(ranges)
    }
    pub(crate) fn contains(&self, track: TrackId, frame: u64) -> bool {
        self.0.get(&track).is_some_and(|ranges| {
            let end = ranges.partition_point(|(start, _)| *start <= frame);
            end > 0 && frame < ranges[end - 1].1
        })
    }
}
