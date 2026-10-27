//! Lossless, bounded conversion from the public Alpha 2 project schema.
//!
//! Both formats used wrapper version 1, so this module only runs after native
//! candidate deserialization fails and only when Alpha 2-only keys identify
//! the document. Divergent musical features are rejected before asset access.
use crate::song::Song;
use serde_json::Value;

const CANDIDATE_DIALECT: &str = "recovered-meters-v1";

fn nonempty(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Object(values)) => !values.is_empty(),
        Some(Value::Array(values)) => !values.is_empty(),
        Some(Value::String(value)) => !value.is_empty(),
        Some(Value::Bool(value)) => *value,
        Some(Value::Number(value)) => value.as_f64().is_some_and(|n| n != 0.0),
    }
}

fn alpha2_signature(song: &Value) -> bool {
    song.get("generation_recipes").is_some()
        || song
            .get("patterns")
            .and_then(Value::as_object)
            .is_some_and(|patterns| patterns.values().any(|p| p.get("aliases").is_some()))
        || song
            .get("order")
            .and_then(Value::as_array)
            .is_some_and(|order| order.iter().any(|o| o.get("section").is_some()))
        || song
            .get("instruments")
            .and_then(Value::as_object)
            .is_some_and(|instruments| instruments.values().any(|i| i.get("phrases").is_some()))
        || song
            .get("patterns")
            .and_then(Value::as_object)
            .is_some_and(|patterns| {
                patterns.values().any(|pattern| {
                    pattern
                        .get("tracks")
                        .and_then(Value::as_object)
                        .is_some_and(|tracks| {
                            tracks.values().any(|rows| {
                                rows.as_object().is_some_and(|rows| {
                                    rows.values().any(|columns| {
                                        columns.as_object().is_some_and(|columns| {
                                            columns.values().any(|cell| {
                                                cell.get("effects").is_some()
                                                    || cell
                                                        .get("phrase")
                                                        .is_some_and(Value::is_object)
                                            })
                                        })
                                    })
                                })
                            })
                        })
                })
            })
}

fn candidate_identity_is_proven(song: &Value) -> bool {
    let nonempty_field = |field: &str| nonempty(song.get(field));
    [
        "phrases",
        "marker_sets",
        "drum_kits",
        "grooves",
        "multisamples",
        "region_catalog",
        "track_mix",
    ]
    .into_iter()
    .any(nonempty_field)
        || has_native_only_effect(song)
        || song
            .get("instruments")
            .and_then(Value::as_object)
            .is_some_and(|instruments| {
                instruments
                    .values()
                    .any(|instrument| instrument.get("kind").is_some_and(|kind| !kind.is_null()))
            })
        || song
            .get("patterns")
            .and_then(Value::as_object)
            .is_some_and(|patterns| {
                patterns.values().any(|pattern| {
                    nonempty(pattern.get("track_aliases"))
                        || pattern
                            .get("generation")
                            .is_some_and(|value| nonempty(Some(value)))
                })
            })
}

// The recovered format predates its later cut/offset commands. Its object-form
// retrigger and glide, release and direction fields cannot be Alpha 2 effects,
// so historical documents containing them already establish the dialect.
fn has_native_only_effect(song: &Value) -> bool {
    song.get("patterns")
        .and_then(Value::as_object)
        .is_some_and(|patterns| {
            patterns.values().any(|pattern| {
                pattern
                    .get("tracks")
                    .and_then(Value::as_object)
                    .is_some_and(|tracks| {
                        tracks.values().any(|rows| {
                            rows.as_object().is_some_and(|rows| {
                                rows.values().any(|columns| {
                                    columns.as_object().is_some_and(|columns| {
                                        columns.values().any(|cell| {
                                            cell.get("effects")
                                                .and_then(Value::as_object)
                                                .is_some_and(|effects| {
                                                    effects
                                                        .get("retrigger")
                                                        .is_some_and(Value::is_object)
                                                        || effects
                                                            .get("glide")
                                                            .is_some_and(Value::is_object)
                                                        || effects
                                                            .get("release")
                                                            .is_some_and(|v| !v.is_null())
                                                        || effects
                                                            .get("direction")
                                                            .is_some_and(|v| !v.is_null())
                                                })
                                        })
                                    })
                                })
                            })
                        })
                    })
            })
        })
}

fn has_nonempty_effects(song: &Value) -> bool {
    song.get("patterns")
        .and_then(Value::as_object)
        .is_some_and(|patterns| {
            patterns.values().any(|pattern| {
                pattern
                    .get("tracks")
                    .and_then(Value::as_object)
                    .is_some_and(|tracks| {
                        tracks.values().any(|rows| {
                            rows.as_object().is_some_and(|rows| {
                                rows.values().any(|columns| {
                                    columns.as_object().is_some_and(|columns| {
                                        columns.values().any(|cell| {
                                            cell.get("effects")
                                                .is_some_and(|value| nonempty(Some(value)))
                                        })
                                    })
                                })
                            })
                        })
                    })
            })
        })
}

/// Schema-1 effect names and JSON shapes overlap even where meanings differ.
/// Reject ambiguous effectful documents before native deserialization can
/// silently assign candidate semantics to Alpha 2 values.
pub(super) fn preflight_effect_semantics(data: &[u8]) -> Result<(), String> {
    let value: Value =
        serde_json::from_slice(data).map_err(|error| format!("Malformed project: {error}"))?;
    let Some(object) = value.as_object() else {
        return Ok(());
    };
    if object.get("format").and_then(Value::as_str) != Some("leviathan-song")
        || object.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Ok(());
    }
    if let Some(dialect) = object.get("dialect").and_then(Value::as_str) {
        return if dialect == CANDIDATE_DIALECT {
            Ok(())
        } else {
            Err("Unsupported project dialect".into())
        };
    }
    let Some(song) = object.get("song") else {
        return Ok(());
    };
    if !has_nonempty_effects(song) || candidate_identity_is_proven(song) {
        return Ok(());
    }
    Err("Ambiguous schema-1 effect semantics: this effectful project has no candidate-only identity marker, and Alpha 2 cut/retrigger/glide fields can share candidate JSON shapes. Open a project with a provable candidate feature or remove the effects in Alpha 2 before migration.".into())
}

pub(super) fn needs_dialect_marker(song: &Song) -> bool {
    let effectful = song.patterns.values().any(|pattern| {
        pattern
            .tracks
            .values()
            .flat_map(|rows| rows.values())
            .flat_map(|columns| columns.values())
            .any(|cell| !cell.effects.is_empty())
    });
    let candidate_only_data = !song.phrases.is_empty()
        || !song.marker_sets.is_empty()
        || !song.drum_kits.is_empty()
        || !song.grooves.is_empty()
        || !song.multisamples.is_empty()
        || !song.region_catalog.is_empty()
        || !song.track_mix.is_empty()
        || song
            .instruments
            .values()
            .any(|instrument| !instrument.kind.is_pitched())
        || song.patterns.values().any(|pattern| {
            !pattern.track_aliases.is_empty()
                || pattern.generation.is_some()
                || pattern
                    .tracks
                    .values()
                    .flat_map(|rows| rows.values())
                    .flat_map(|columns| columns.values())
                    .any(|cell| {
                        cell.effects.retrigger.is_some()
                            || cell.effects.glide.is_some()
                            || cell.effects.release.is_some()
                            || cell.effects.direction.is_some()
                    })
        });
    effectful && !candidate_only_data
}

fn convert(mut song: Value) -> Result<Song, String> {
    if let Some(instruments) = song.get_mut("instruments").and_then(Value::as_object_mut) {
        for instrument in instruments.values_mut() {
            if nonempty(instrument.get("phrases")) {
                return Err("Alpha 2 project cannot be migrated losslessly: instrument-owned phrases cannot be mapped to candidate global phrases without changing ownership or trigger-cycle semantics".into());
            }
            if let Some(object) = instrument.as_object_mut() {
                object.remove("phrases");
            }
        }
    }

    if let Some(patterns) = song.get_mut("patterns").and_then(Value::as_object_mut) {
        for pattern in patterns.values_mut() {
            let Some(object) = pattern.as_object_mut() else {
                return Err("Malformed Alpha 2 project: pattern must be an object".into());
            };
            if let Some(aliases) = object.remove("aliases") {
                if object.contains_key("track_aliases") {
                    return Err("Malformed Alpha 2 project: conflicting alias fields".into());
                }
                // Alpha 2 BlockRef and candidate TrackAlias have identical
                // pattern/track identity and keep the same source track.
                if nonempty(Some(&aliases)) {
                    object.insert("track_aliases".into(), aliases);
                }
            }
            if let Some(tracks) = object.get_mut("tracks").and_then(Value::as_object_mut) {
                for rows in tracks.values_mut() {
                    let Some(rows) = rows.as_object_mut() else {
                        return Err(
                            "Malformed Alpha 2 project: track pattern must be an object".into()
                        );
                    };
                    for columns in rows.values_mut() {
                        let Some(columns) = columns.as_object_mut() else {
                            return Err(
                                "Malformed Alpha 2 project: pattern row must be an object".into()
                            );
                        };
                        for cell in columns.values_mut() {
                            let Some(cell) = cell.as_object_mut() else {
                                return Err(
                                    "Malformed Alpha 2 project: cell must be an object".into()
                                );
                            };
                            if cell
                                .get("phrase")
                                .is_some_and(|value| nonempty(Some(value)))
                            {
                                return Err("Alpha 2 project cannot be migrated losslessly: phrase triggers and cycle counts have no equivalent candidate representation".into());
                            }
                            if let Some(effects) = cell.remove("effects") {
                                let empty = effects.as_object().is_some_and(|fields| {
                                    fields.values().all(|value| !nonempty(Some(value)))
                                });
                                if !empty && nonempty(Some(&effects)) {
                                    return Err("Alpha 2 project cannot be migrated losslessly: effect timing, units, or glide targets differ from candidate effects".into());
                                }
                            }
                            cell.remove("phrase");
                        }
                    }
                }
            }
        }
    }

    serde_json::from_value(song)
        .map_err(|error| format!("Malformed or unsupported Alpha 2 project: {error}"))
}

/// Returns `None` for non-Alpha-2 input. The caller supplies bytes already
/// bounded by project I/O and invokes this only after native parsing fails.
pub(super) fn migrate_alpha2(data: &[u8]) -> Result<Option<Song>, String> {
    let value: Value =
        serde_json::from_slice(data).map_err(|error| format!("Malformed project: {error}"))?;
    let Some(object) = value.as_object() else {
        return Ok(None);
    };
    if object.get("format").and_then(Value::as_str) != Some("leviathan-song")
        || object.get("version").and_then(Value::as_u64) != Some(1)
    {
        return Ok(None);
    }
    let Some(song) = object.get("song") else {
        return Ok(None);
    };
    if !alpha2_signature(song) {
        return Ok(None);
    }
    let Some(mut song) = song.as_object().cloned() else {
        return Err("Malformed Alpha 2 project: song must be an object".into());
    };
    // Empty Alpha 2 recipe metadata has exactly the candidate's default meaning.
    for field in ["generation_recipes"] {
        if song.get(field).is_some_and(|value| !nonempty(Some(value))) {
            song.remove(field);
        }
    }
    convert(Value::Object(song)).map(Some)
}
