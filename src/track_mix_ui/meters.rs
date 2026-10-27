//! Presentation-only render-ahead meters. Raw reports are never smoothed or
//! written back to the Engine, document, live-mix mailbox or transport commands.
use crate::{
    meter_windows::MeterReport,
    meters::{MeterLayout, TrackMeterAvailability, MAX_TRACKS, REPORT_CAPACITY, WINDOW_FRAMES},
    song::TrackId,
    transport::MeterPoll,
};
use eframe::egui::{self, Color32, RichText};
use std::{collections::BTreeMap, sync::Arc};

const FLOOR_DB: f32 = -60.0;
const HOLD_SECONDS: f64 = 0.250;
const DECAY_DB_PER_SECOND: f64 = 24.0;

#[derive(Clone, Copy, Debug)]
struct DisplayPeak {
    db: f32,
    held_until: f64,
    updated: f64,
}
impl Default for DisplayPeak {
    fn default() -> Self {
        Self {
            db: FLOOR_DB,
            held_until: 0.0,
            updated: 0.0,
        }
    }
}
fn db(peak: f32) -> f32 {
    if peak > 0.0 {
        (20.0 * peak.log10()).max(FLOOR_DB)
    } else {
        FLOOR_DB
    }
}
impl DisplayPeak {
    fn advance(&mut self, now: f64) {
        let now = if now.is_finite() {
            now.max(self.updated)
        } else {
            self.updated
        };
        let from = self.updated.max(self.held_until);
        if now > from {
            self.db = (self.db - ((now - from) * DECAY_DB_PER_SECOND) as f32).max(FLOOR_DB);
        }
        self.updated = now;
    }
    fn observe(&mut self, value: f32, now: f64) {
        self.advance(now);
        let target = db(value);
        if target >= self.db {
            self.db = target;
            self.held_until = self.updated + HOLD_SECONDS;
        }
    }
}

struct InspectionSnapshot {
    text: String,
    generation: u64,
    report_index: u64,
    emitted_start: u64,
    emitted_end: u64,
}

#[derive(Default)]
pub(super) struct MeterView {
    generation: Option<u64>,
    layout: Option<Arc<MeterLayout>>,
    report: Option<MeterReport>,
    reason: String,
    dropped: u64,
    received: u64,
    coalesced: u64,
    last_poll_reports: usize,
    master: [DisplayPeak; 2],
    tracks: BTreeMap<TrackId, [DisplayPeak; 2]>,
    raw: String,
    inspection: Option<InspectionSnapshot>,
}
impl MeterView {
    pub(super) fn clear(&mut self, reason: impl Into<String>) {
        let reason = reason.into();
        if self.report.is_none()
            && self.layout.is_none()
            && self.generation.is_none()
            && self.reason == reason
        {
            return;
        }
        *self = Self {
            reason,
            ..Self::default()
        };
    }
    pub(super) fn accept(&mut self, poll: MeterPoll, now: f64) {
        let prior_metadata = (self.dropped, self.last_poll_reports);
        if self.generation != Some(poll.generation)
            || self.layout.as_ref().is_some_and(|old| {
                poll.layout
                    .as_ref()
                    .is_none_or(|new| !Arc::ptr_eq(old, new))
            })
        {
            self.clear("Waiting for a complete window");
        }
        self.generation = Some(poll.generation);
        self.layout = poll.layout;
        self.dropped = poll.dropped_reports;
        self.last_poll_reports = poll.reports.len();
        if let Some(reason) = poll.disabled_reason {
            self.inspection = None;
            self.report = None;
            self.tracks.clear();
            self.master = Default::default();
            let changed =
                self.reason != reason || prior_metadata != (self.dropped, self.last_poll_reports);
            self.reason = reason;
            if changed || self.raw.is_empty() {
                self.rebuild_raw();
            }
            return;
        }
        if poll.reports.len() > REPORT_CAPACITY
            || self
                .layout
                .as_ref()
                .is_some_and(|l| l.tracks.len() > MAX_TRACKS)
        {
            self.clear("Meter report exceeds the bounded display contract");
            return;
        }
        if let Some(latest) = poll.reports.last() {
            let valid = self.layout.as_ref().is_some_and(|layout| {
                !latest.partial
                    && latest
                        .emitted_end_exclusive
                        .checked_sub(latest.emitted_start)
                        == Some(WINDOW_FRAMES)
                    && latest.track_peaks().len() == layout.tracks.len()
                    && latest
                        .master_peak
                        .iter()
                        .all(|v| v.is_finite() && *v >= 0.0)
                    && latest
                        .track_peaks()
                        .iter()
                        .flatten()
                        .flatten()
                        .all(|v| v.is_finite() && *v >= 0.0)
            });
            if !valid {
                self.clear("Meter window unavailable: invalid or incomplete report");
                return;
            }
            self.received = self.received.saturating_add(poll.reports.len() as u64);
            self.coalesced = self
                .coalesced
                .saturating_add(poll.reports.len().saturating_sub(1) as u64);
            self.report = Some(*latest);
            self.reason.clear();
            for (display, value) in self.master.iter_mut().zip(latest.master_peak) {
                display.observe(value, now);
            }
            for (slot, row) in self.layout.as_ref().unwrap().tracks.iter().enumerate() {
                if let Some(values) = latest.track_peaks()[slot] {
                    let displays = self.tracks.entry(row.track).or_default();
                    for (display, value) in displays.iter_mut().zip(values) {
                        display.observe(value, now);
                    }
                } else {
                    self.tracks.remove(&row.track);
                }
            }
            self.rebuild_raw();
        } else if self.report.is_none() {
            let reason = if self.layout.is_some() {
                "Waiting for a complete 50 ms window"
            } else {
                "Waiting for worker layout"
            };
            if self.reason != reason
                || self.raw.is_empty()
                || prior_metadata != (self.dropped, self.last_poll_reports)
            {
                self.reason = reason.into();
                self.rebuild_raw();
            }
        } else if prior_metadata != (self.dropped, self.last_poll_reports) {
            // Queue eviction/coalescing metadata can change even without a new
            // visible report. Do not turn an empty poll into a zero measurement.
            self.rebuild_raw();
        }
        for peak in &mut self.master {
            peak.advance(now);
        }
        for pair in self.tracks.values_mut() {
            for peak in pair {
                peak.advance(now);
            }
        }
    }
    fn rebuild_raw(&mut self) {
        let slots: Vec<_> = self.layout.as_ref().map(|layout| layout.tracks.iter().enumerate().map(|(slot,row)| {
            serde_json::json!({"slot":slot,"track":row.track,"availability":match &row.availability {
                TrackMeterAvailability::Available=>"available; None means a nonfinite meter-only subtotal in this window",
                TrackMeterAvailability::SharedLegacy{..}=>"whole track unavailable: shared legacy sampler"
            },"shared_legacy_instrument_count":match &row.availability {TrackMeterAvailability::Available=>0,TrackMeterAvailability::SharedLegacy{instruments}=>instruments.len()}})
        }).collect()).unwrap_or_default();
        self.raw = serde_json::to_string_pretty(&serde_json::json!({
            "authority":"latest wholly enqueued worker render-ahead window; not device/heard output",
            "generation":self.generation,"window_frames":WINDOW_FRAMES,"queue_capacity":REPORT_CAPACITY,
            "queue_evictions":self.dropped,"reports_received":self.received,
            "ui_display_coalesced":self.coalesced,"last_poll_reports":self.last_poll_reports,
            "unavailable_reason":self.reason,"tracks_unavailable":self.layout.as_ref().and_then(|l|l.tracks_unavailable.as_deref()),
            "latest_report":self.report,"master_f32_bits":self.report.map(|r|r.master_peak.map(f32::to_bits)),
            "track_f32_bits":self.report.map(|r|r.track_peaks().iter().map(|p|p.map(|v|v.map(f32::to_bits))).collect::<Vec<_>>()),
            "track_slot_mapping":slots
        })).expect("bounded finite meter metadata");
    }
    fn values(&self, track: Option<TrackId>) -> Result<[f32; 2], String> {
        if let Some(track) = track {
            let Some(layout) = &self.layout else {
                return Err(self.empty_reason());
            };
            if let Some(reason) = &layout.tracks_unavailable {
                return Err(format!("Track meters unavailable: {reason}"));
            }
            let Some((slot, row)) = layout
                .tracks
                .iter()
                .enumerate()
                .find(|(_, r)| r.track == track)
            else {
                return Err("No scheduled ON contribution".into());
            };
            if matches!(
                row.availability,
                TrackMeterAvailability::SharedLegacy { .. }
            ) {
                return Err("Whole track unavailable: shared legacy sampler".into());
            }
            let Some(report) = self.report else {
                return Err(self.empty_reason());
            };
            report.track_peaks()[slot]
                .ok_or_else(|| "Window unavailable: nonfinite meter-only subtotal".into())
        } else {
            self.report
                .map(|r| r.master_peak)
                .ok_or_else(|| self.empty_reason())
        }
    }
    fn empty_reason(&self) -> String {
        if self.reason.is_empty() {
            "No complete worker window".into()
        } else {
            self.reason.clone()
        }
    }
    fn paint(&self, ui: &egui::Ui, rect: egui::Rect, track: Option<TrackId>) {
        let result = self.values(track);
        let painter = ui.painter();
        let background = Color32::from_rgb(17, 29, 34);
        for channel in 0..2 {
            let r = egui::Rect::from_min_size(
                rect.min + egui::vec2(0.0, channel as f32 * 5.0),
                egui::vec2(rect.width(), 3.0),
            );
            painter.rect_filled(r, 0.0, background);
            if result.is_ok() {
                let pair = track
                    .and_then(|id| self.tracks.get(&id))
                    .unwrap_or(&self.master);
                let level = pair[channel].db;
                let width = r.width() * ((level - FLOOR_DB) / -FLOOR_DB).clamp(0.0, 1.0);
                if width > 0.0 {
                    painter.rect_filled(
                        egui::Rect::from_min_size(r.min, egui::vec2(width, r.height())),
                        0.0,
                        if level > 0.0 {
                            Color32::LIGHT_RED
                        } else {
                            super::AQUA
                        },
                    );
                }
            }
        }
        if let Err(reason) = result {
            let text = if reason.contains("shared legacy") {
                "unavailable · shared legacy"
            } else if reason.contains("No scheduled") {
                "no scheduled ON"
            } else if reason.contains("nonfinite") {
                "window unavailable"
            } else {
                "—"
            };
            painter.text(
                rect.center(),
                egui::Align2::CENTER_CENTER,
                text,
                egui::FontId::proportional(9.0),
                Color32::GRAY,
            );
        }
    }
    fn tooltip(&self, track: Option<TrackId>) -> String {
        match self.values(track) {
            Ok(v)=>format!("Measured sample peak L {:e} / R {:e}\nL {} / R {} dBFS\nWorker render-ahead, not heard. Above 0 dBFS is not a device-clipping diagnosis. Bars: display-only 250 ms hold, 24 dB/s decay to -60 dB; raw values unchanged.",v[0],v[1],if v[0]==0.0{"-inf (measured zero)".into()}else{format!("{:.2}",20.0*v[0].log10())},if v[1]==0.0{"-inf (measured zero)".into()}else{format!("{:.2}",20.0*v[1].log10())}),
            Err(reason)=>format!("{reason}\nNo zero level is inferred. Worker render-ahead, not heard.")
        }
    }
    pub(super) fn row(
        &self,
        ui: &egui::Ui,
        response: egui::Response,
        track: TrackId,
        identity: &str,
    ) {
        let rect = egui::Rect::from_min_size(
            egui::pos2(response.rect.left() + 4.0, response.rect.bottom() - 9.0),
            egui::vec2(response.rect.width() - 8.0, 8.0),
        );
        self.paint(ui, rect, Some(track));
        let tooltip = format!("{identity}\n{}", self.tooltip(Some(track)));
        response
            .on_hover_text(&tooltip)
            .on_disabled_hover_text(tooltip);
    }
    pub(super) fn master(&self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("MASTER").small().strong());
            let (r, response) =
                ui.allocate_exact_size(egui::vec2(128.0, 12.0), egui::Sense::hover());
            self.paint(ui, r, None);
            response.on_hover_text(self.tooltip(None));
            ui.label(
                RichText::new(if self.report.is_some() {
                    "Worker sample peaks · render-ahead, not heard"
                } else if self.reason.starts_with("Waiting") || self.reason.is_empty() {
                    "Worker peaks · waiting for a complete window"
                } else {
                    "Worker peaks · unavailable (details)"
                })
                .small(),
            );
        });
    }
    pub(super) fn contributing(&self, ui: &mut egui::Ui) {
        if let Some(layout) = &self.layout {
            ui.label(
                RichText::new(format!(
                    "Live contributing tracks · {} read-only meter rows",
                    layout.tracks.len()
                ))
                .small(),
            );
            for row in &layout.tracks {
                ui.horizontal(|ui| {
                    let response =
                        ui.add_sized([240.0, 34.0], egui::Label::new(format!("T{}", row.track.0)));
                    self.row(
                        ui,
                        response,
                        row.track,
                        &format!("Track {} · read-only compiled contributor", row.track.0),
                    );
                });
            }
            if let Some(reason) = &layout.tracks_unavailable {
                ui.weak(reason);
            }
        }
    }
    pub(super) fn details(&mut self, ui: &mut egui::Ui) {
        ui.label("Raw worker meters: fixed 2,400 emitted-frame (50 ms) windows; four-report queue. Partial windows at Stop/seek/error/live end are omitted, not padded with zeros. Late owner binding can omit an incomplete leading window; observation_start_emitted records that start. Song-frame endpoints are not asset source frames; loop wraps can occur inside a window.");
        ui.label("Peaks are actual post-balance AND post-fixed-master-0.5 sample amplitudes, not loudness or true peak. Master is final stereo PCM. Shared legacy ownership makes the entire affected track unavailable, never a partial subtotal. For an otherwise available track, a null peak marks a nonfinite meter-only subtotal somewhere in that window, not measured zero.");
        ui.label("Bars alone use a presentation-only 250 ms hold and 24 dB/s decay to -60 dBFS. Queue evictions are session-cumulative; received/display-coalesced counts are since this display binding or generation reset. Raw latest report is unchanged. This does not measure callback consumption, devices or heard output.");
        if !self.reason.is_empty() {
            ui.weak(&self.reason);
        }
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.report.is_some(),
                    egui::Button::new("Snapshot latest report"),
                )
                .clicked()
            {
                let report = self.report.unwrap();
                self.inspection = Some(InspectionSnapshot {
                    text: self.raw.clone(),
                    generation: self.generation.unwrap(),
                    report_index: report.report_index,
                    emitted_start: report.emitted_start,
                    emitted_end: report.emitted_end_exclusive,
                });
            }
            if ui
                .add_enabled(
                    self.inspection.is_some(),
                    egui::Button::new("Follow latest"),
                )
                .clicked()
            {
                self.inspection = None;
            }
        });
        if let Some(snapshot) = &self.inspection {
            ui.label(RichText::new(format!("INSPECTION SNAPSHOT #{} · generation {} · emitted [{}..{}) · bars and collection continue",snapshot.report_index,snapshot.generation,snapshot.emitted_start,snapshot.emitted_end)).small().color(super::AQUA));
        } else {
            ui.label(
                RichText::new("Following latest raw report · bars and collection continue").small(),
            );
        }
        let mut raw = self
            .inspection
            .as_ref()
            .map_or(self.raw.as_str(), |s| s.text.as_str());
        ui.add(
            egui::TextEdit::multiline(&mut raw)
                .id(egui::Id::new("track-mix-meter-raw"))
                .font(egui::TextStyle::Monospace)
                .desired_width(f32::INFINITY)
                .cursor_at_end(false)
                .desired_rows(4),
        );
        if let Some(layout) = &self.layout {
            for row in &layout.tracks {
                if let TrackMeterAvailability::SharedLegacy { instruments } = &row.availability {
                    egui::CollapsingHeader::new(format!(
                        "T{} shared legacy: {} exact instrument IDs",
                        row.track.0,
                        instruments.len()
                    ))
                    .id_salt(("meter-legacy", row.track.0))
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt(("meter-legacy-ids", row.track.0))
                            .max_height(110.0)
                            .show_rows(ui, 18.0, instruments.len(), |ui, range| {
                                for index in range {
                                    ui.monospace(format!("I{}", instruments[index].0));
                                }
                            });
                    });
                }
            }
        }
    }
    #[cfg(any(test, feature = "ui-capture"))]
    pub(super) fn capture(&self) -> serde_json::Value {
        serde_json::json!({"generation":self.generation,"layout":self.layout.as_deref(),"report":self.report,"reason":self.reason,"dropped_reports":self.dropped,"received_reports":self.received,"display_coalesced":self.coalesced,"last_poll_reports":self.last_poll_reports,"raw_text":self.raw,"inspection_snapshot":self.inspection.as_ref().map(|s|serde_json::json!({"text":s.text,"generation":s.generation,"report_index":s.report_index,"emitted_start":s.emitted_start,"emitted_end_exclusive":s.emitted_end})),"master_display_db":self.master.map(|p|p.db),"track_display_db":self.tracks.iter().map(|(id,p)|(id.0,p.map(|p|p.db))).collect::<BTreeMap<_,_>>()})
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn hold_decay_uses_only_injected_ui_time_and_never_overstates_zero_as_missing() {
        let mut p = DisplayPeak::default();
        p.observe(1.0, 1.0);
        p.advance(1.2);
        assert_eq!(p.db, 0.0);
        p.advance(1.5);
        assert_eq!(p.db, -6.0);
        p.observe(0.0, 1.5);
        assert_eq!(p.db, -6.0);
        p.advance(4.0);
        assert_eq!(p.db, FLOOR_DB);
        p.observe(2.0, 5.0);
        assert!(p.db > 0.0);
        let old = p.db;
        p.advance(f64::NAN);
        assert_eq!(p.db, old);
        p.advance(1.0);
        assert_eq!(p.db, old);
        assert_eq!(db(f32::from_bits(1)), FLOOR_DB);
    }
    #[test]
    fn empty_and_shared_legacy_are_not_measured_zero_and_generation_change_clears() {
        let mut view = MeterView::default();
        view.clear("Stopped");
        assert!(view.values(None).is_err());
        let layout = Arc::new(MeterLayout {
            tracks: vec![crate::meters::TrackMeterRow {
                track: TrackId(1),
                availability: TrackMeterAvailability::SharedLegacy {
                    instruments: vec![crate::song::InstrumentId(1)],
                },
            }],
            tracks_unavailable: None,
        });
        view.accept(
            MeterPoll {
                generation: 1,
                dropped_reports: 3,
                layout: Some(layout.clone()),
                reports: vec![],
                disabled_reason: None,
            },
            0.0,
        );
        assert!(view
            .values(Some(TrackId(1)))
            .unwrap_err()
            .contains("Whole track"));
        assert!(view
            .values(Some(TrackId(2)))
            .unwrap_err()
            .contains("No scheduled"));
        view.master[0].observe(1.0, 0.0);
        view.accept(
            MeterPoll {
                generation: 2,
                dropped_reports: 4,
                layout: Some(layout),
                reports: vec![],
                disabled_reason: None,
            },
            0.1,
        );
        assert_eq!(view.master[0].db, FLOOR_DB);
        assert_eq!(view.generation, Some(2));
        assert!(view.report.is_none());
        view.clear("Owner changed");
        assert!(view.layout.is_none());
        assert!(view.raw.is_empty());
    }
}
