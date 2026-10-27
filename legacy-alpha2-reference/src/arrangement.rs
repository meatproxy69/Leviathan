//! Arrangement compilation keeps persistent aliases separate from render-time copies.
use crate::song::*;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ArrangementInfo {
    /// Preserve the original DSP path for alias/section-only songs.
    pub isolated_voices: bool,
    pub aliases: Vec<(BlockRef, BlockRef)>,
    pub order: Vec<Occurrence>,
}

pub(crate) fn materialize(song: &Song) -> Result<(Song, ArrangementInfo), String> {
    song.validate()?;
    // Bound played content before expanding any aliases or muted variants. Unused
    // pool entries and muted source cells must never be copied into render content.
    let mut populated = 0usize;
    for occurrence in &song.order {
        for track in &song.track_order {
            let count = if occurrence.muted_tracks.contains(track) {
                song.tracks[track].note_columns as usize
            } else {
                song.block(occurrence.pattern, *track)?.map_or(0, |block| {
                    block
                        .values()
                        .flat_map(|columns| columns.values())
                        .filter(|cell| **cell != Cell::default())
                        .take(32769 - populated)
                        .count()
                })
            };
            populated = populated
                .checked_add(count)
                .ok_or("Arrangement cell count overflow")?;
            if populated > 32768 {
                return Err(
                    "Song exceeds 32768 populated cells after arrangement resolution".into(),
                );
            }
        }
    }
    let mut result = song.clone();
    result.patterns.clear();
    let mut aliases = Vec::new();
    for (id, pattern) in &song.patterns {
        for track in pattern.aliases.keys() {
            aliases.push((
                BlockRef {
                    pattern: *id,
                    track: *track,
                },
                song.block_source(*id, *track)?,
            ));
        }
    }
    let mut variants = BTreeMap::new();
    let mut reserved: std::collections::BTreeSet<_> = song.patterns.keys().copied().collect();
    for occurrence in &mut result.order {
        let key = (occurrence.pattern, occurrence.muted_tracks.clone());
        let id = if let Some(id) = variants.get(&key) {
            *id
        } else {
            let id = if occurrence.muted_tracks.is_empty() {
                occurrence.pattern
            } else {
                let id = (1..)
                    .map(PatternId)
                    .find(|id| !reserved.contains(id))
                    .ok_or("Pattern IDs exhausted")?;
                reserved.insert(id);
                id
            };
            let original = &song.patterns[&occurrence.pattern];
            let mut pattern = Pattern {
                name: original.name.clone(),
                rows: original.rows,
                tracks: BTreeMap::new(),
                aliases: BTreeMap::new(),
            };
            for track in &song.track_order {
                let block = if occurrence.muted_tracks.contains(track) {
                    BTreeMap::from([(
                        0,
                        (0..song.tracks[track].note_columns)
                            .map(|column| {
                                (
                                    column,
                                    Cell {
                                        effects: StepEffects {
                                            cut: Some(0.0),
                                            ..Default::default()
                                        },
                                        ..Default::default()
                                    },
                                )
                            })
                            .collect(),
                    )])
                } else {
                    song.block(occurrence.pattern, *track)?
                        .map(|block| {
                            block
                                .iter()
                                .filter_map(|(row, columns)| {
                                    let columns: BTreeMap<_, _> = columns
                                        .iter()
                                        .filter(|(_, cell)| **cell != Cell::default())
                                        .map(|(column, cell)| (*column, cell.clone()))
                                        .collect();
                                    (!columns.is_empty()).then_some((*row, columns))
                                })
                                .collect()
                        })
                        .unwrap_or_default()
                };
                if !block.is_empty() {
                    pattern.tracks.insert(*track, block);
                }
            }
            result.patterns.insert(id, pattern);
            variants.insert(key, id);
            id
        };
        occurrence.pattern = id;
        occurrence.muted_tracks.clear();
    }
    Ok((
        result,
        ArrangementInfo {
            isolated_voices: false,
            aliases,
            order: song.order.clone(),
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn render_projection_excludes_unused_alias_pool_and_muted_source_cells() {
        let mut song = example_song();
        song.order.truncate(1);
        for id in 3..100 {
            let mut p = song.patterns[&PatternId(1)].clone();
            p.tracks.clear();
            p.aliases.insert(
                TrackId(1),
                BlockRef {
                    pattern: PatternId(1),
                    track: TrackId(1),
                },
            );
            song.patterns.insert(PatternId(id), p);
        }
        song.order[0].muted_tracks.insert(TrackId(1));
        let (content, _) = materialize(&song).unwrap();
        assert_eq!(content.patterns.len(), 1);
        let p = &content.patterns[&content.order[0].pattern];
        assert!(p.aliases.is_empty());
        assert_eq!(p.tracks[&TrackId(1)].len(), 1);
        assert!(p.tracks[&TrackId(1)][&0]
            .values()
            .all(|c| c.note == Note::Empty && c.effects.cut == Some(0.0)));
        assert_eq!(song.patterns.len(), 99);
    }
}
