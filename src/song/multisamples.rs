//! Explicit exclusive melodic note/velocity routing. No audio or runtime counters.
use super::*;
impl Song {
    pub(super) fn validate_multisamples(&self) -> Result<(), String> {
        if self.multisamples.len() > 64 {
            return Err("song exceeds 64 multisample maps".into());
        }
        let mut total = 0usize;
        for (id, map) in &self.multisamples {
            total = total.saturating_add(map.zones.len());
            if id.0 == 0
                || map.name.trim().is_empty()
                || map.name.len() > 256
                || map.zones.len() > 128
                || total > 4096
            {
                return Err(
                    "invalid multisample ID/name or exceeds 128 zones/map or 4096 zones/song"
                        .into(),
                );
            }
        }
        // Structural caps precede all pair scans, including unused maps.
        for map in self.multisamples.values() {
            for (id, zone) in &map.zones {
                if id.0 == 0
                    || zone.name.trim().is_empty()
                    || zone.name.len() > 128
                    || zone.note_min > zone.note_max
                    || zone.note_max > 127
                    || zone.velocity_min > zone.velocity_max
                    || zone.velocity_max > 127
                    || zone.root_note > 127
                    || !self.regions.contains_key(&zone.region)
                {
                    return Err(
                        "invalid keyzone ID/name/note/velocity/root or unknown region".into(),
                    );
                }
                for (other_id, other) in map
                    .zones
                    .range((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
                {
                    if zone.note_min <= other.note_max
                        && other.note_min <= zone.note_max
                        && zone.velocity_min <= other.velocity_max
                        && other.velocity_min <= zone.velocity_max
                    {
                        return Err(format!("Keyzones {} and {} overlap in inclusive note AND velocity ranges; no layering or priority fallback", id.0, other_id.0));
                    }
                }
            }
        }
        for instrument in self.instruments.values() {
            if let InstrumentKind::Multisample { map } = instrument.kind {
                if !self.multisamples.contains_key(&map)
                    || instrument.sample.is_some()
                    || instrument.region.is_some()
                {
                    return Err("multisample instrument references unknown map or conflicts with sample/region".into());
                }
            }
        }
        Ok(())
    }
    /// Select from a validated document. The actual pre-source ON f32 value selects
    /// a bin; audible gain is not quantized. Gaps are errors, never silent fallback.
    pub fn select_keyzone(
        &self,
        map: MultisampleId,
        note: u8,
        velocity: f32,
    ) -> Result<(KeyZoneId, &KeyZone, u8), String> {
        if note > 127 || !velocity.is_finite() || !(0.0..=1.0).contains(&velocity) {
            return Err("Keyzone selection requires MIDI0..127 and finite ON velocity0..1".into());
        }
        let map_value = self
            .multisamples
            .get(&map)
            .ok_or("unknown multisample map")?;
        if map_value.zones.len() > 128 {
            return Err("multisample exceeds 128 zones".into());
        }
        let bin = (f64::from(velocity) * 127.0).round() as u8;
        let mut selected = None;
        for (&id, zone) in &map_value.zones {
            if zone.note_min <= note
                && note <= zone.note_max
                && zone.velocity_min <= bin
                && bin <= zone.velocity_max
            {
                if selected.is_some() {
                    return Err("Ambiguous overlapping keyzone selection".into());
                }
                selected = Some((id, zone, bin));
            }
        }
        selected.ok_or_else(|| format!("Multisample {} has no keyzone for note {note}, velocity bin {bin}; map the gap explicitly (no fallback)", map.0))
    }
    pub fn upsert_multisample(
        &mut self,
        id: MultisampleId,
        map: MultisampleInstrument,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.multisamples.insert(id, map);
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn set_keyzones(
        &mut self,
        id: MultisampleId,
        zones: BTreeMap<KeyZoneId, KeyZone>,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate
            .multisamples
            .get_mut(&id)
            .ok_or("unknown multisample map")?
            .zones = zones;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn remove_multisample(&mut self, id: MultisampleId) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate
            .multisamples
            .remove(&id)
            .ok_or("unknown multisample map")?;
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    /// Some is an explicit source-kind switch. None detaches only this new kind;
    /// existing Pitched/DrumKit assignments are left exactly unchanged.
    pub fn assign_multisample(
        &mut self,
        instrument: InstrumentId,
        map: Option<MultisampleId>,
    ) -> Result<(), String> {
        let mut candidate = self.clone();
        let target = candidate
            .instruments
            .get_mut(&instrument)
            .ok_or("unknown instrument")?;
        if let Some(map) = map {
            target.kind = InstrumentKind::Multisample { map };
            target.sample = None;
            target.region = None;
        } else if matches!(target.kind, InstrumentKind::Multisample { .. }) {
            target.kind = InstrumentKind::Pitched;
        }
        candidate.validate()?;
        *self = candidate;
        Ok(())
    }
    pub fn create_multisample_instrument(
        &mut self,
        name: String,
    ) -> Result<(InstrumentId, MultisampleId), String> {
        let instrument = InstrumentId(
            self.instruments
                .keys()
                .next_back()
                .map_or(Some(1), |id| id.0.checked_add(1))
                .ok_or("Instrument ID allocation overflow")?,
        );
        let map = MultisampleId(
            self.multisamples
                .keys()
                .next_back()
                .map_or(Some(1), |id| id.0.checked_add(1))
                .ok_or("Multisample ID allocation overflow")?,
        );
        let mut candidate = self.clone();
        candidate.multisamples.insert(
            map,
            MultisampleInstrument {
                name: name.clone(),
                zones: BTreeMap::new(),
            },
        );
        candidate.instruments.insert(
            instrument,
            Instrument {
                name,
                kind: InstrumentKind::Multisample { map },
                sample: None,
                region: None,
            },
        );
        candidate.validate()?;
        *self = candidate;
        Ok((instrument, map))
    }
}
