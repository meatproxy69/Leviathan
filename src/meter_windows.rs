//! Fixed 2,400-emitted-frame sample-peak windows shared by live and offline reads.
//! This accumulator never changes audio. Track sums come from the Engine's actual
//! post-balance, post-fixed-master f32 contribution order; master uses final PCM.
use crate::meters::{MeterBlock, MeterLayout, TrackMeterAvailability, MAX_TRACKS, WINDOW_FRAMES};
use serde::{ser::SerializeStruct, Serialize, Serializer};

/// Raw render-ahead observation, not loudness, true peak, device output or hearing.
/// Song-frame endpoints need not form a continuous range: an automatic loop can wrap
/// within a window. A partial offline natural-end window is explicitly identified.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MeterReport {
    pub report_index: u64,
    /// Generation-local worker emitted offset where observation began. A late
    /// bind omits the incomplete leading window rather than calling it frame 0.
    pub observation_start_emitted: u64,
    pub emitted_start: u64,
    pub emitted_end_exclusive: u64,
    pub first_song_frame: u64,
    pub last_song_frame_end_exclusive: u64,
    pub wrap_count: u64,
    pub first_mix_revision: u64,
    pub last_mix_revision: u64,
    pub master_peak: [f32; 2],
    pub partial: bool,
    track_count: usize,
    track_peaks: [Option<[f32; 2]>; MAX_TRACKS],
}
impl MeterReport {
    pub fn frames(&self) -> u64 {
        self.emitted_end_exclusive - self.emitted_start
    }
    /// Aligned with MeterLayout::tracks. None is unavailable, never silence: the
    /// layout identifies shared legacy ownership; an otherwise available row had
    /// a nonfinite meter-only subtotal somewhere in this entire window.
    pub fn track_peaks(&self) -> &[Option<[f32; 2]>] {
        &self.track_peaks[..self.track_count]
    }
}
impl Serialize for MeterReport {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut state = serializer.serialize_struct("MeterReport", 13)?;
        state.serialize_field("report_index", &self.report_index)?;
        state.serialize_field("observation_start_emitted", &self.observation_start_emitted)?;
        state.serialize_field("emitted_start", &self.emitted_start)?;
        state.serialize_field("emitted_end_exclusive", &self.emitted_end_exclusive)?;
        state.serialize_field("first_song_frame", &self.first_song_frame)?;
        state.serialize_field(
            "last_song_frame_end_exclusive",
            &self.last_song_frame_end_exclusive,
        )?;
        state.serialize_field("wrap_count", &self.wrap_count)?;
        state.serialize_field("first_mix_revision", &self.first_mix_revision)?;
        state.serialize_field("last_mix_revision", &self.last_mix_revision)?;
        state.serialize_field("master_peak", &self.master_peak)?;
        state.serialize_field("partial", &self.partial)?;
        state.serialize_field("frames", &self.frames())?;
        state.serialize_field("track_peaks", &self.track_peaks())?;
        state.end()
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterWindowError {
    TooManyTracks,
    CounterOverflow,
    InvalidBlock,
}
impl std::fmt::Display for MeterWindowError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::TooManyTracks => "Meter layout exceeds 64 rows",
            Self::CounterOverflow => {
                "Meter counter exhausted; telemetry disabled without stopping audio"
            }
            Self::InvalidBlock => "Meter block is unavailable or contains invalid final PCM",
        })
    }
}
impl std::error::Error for MeterWindowError {}

/// Fixed-size worker/offline scratch; neither construction nor observation needs
/// per-frame allocation. Dropping/resetting a pending window never fabricates a
/// zero-padded report. The caller decides whether a natural-end partial is kept.
pub struct MeterWindows {
    available: [bool; MAX_TRACKS],
    track_count: usize,
    emitted: u64,
    observation_start: u64,
    skip_until: u64,
    reports: u64,
    pending: Option<MeterReport>,
}
impl MeterWindows {
    pub fn new(layout: &MeterLayout) -> Result<Self, MeterWindowError> {
        if layout.tracks.len() > MAX_TRACKS {
            return Err(MeterWindowError::TooManyTracks);
        }
        let mut available = [false; MAX_TRACKS];
        for (slot, row) in layout.tracks.iter().enumerate() {
            available[slot] = matches!(row.availability, TrackMeterAvailability::Available);
        }
        Ok(Self {
            available,
            track_count: layout.tracks.len(),
            emitted: 0,
            observation_start: 0,
            skip_until: 0,
            reports: 0,
            pending: None,
        })
    }
    /// A manual seek starts a new generation and emitted interval. Automatic loop
    /// restarts deliberately do not call reset, so their windows stay monotonic.
    pub fn reset(&mut self) {
        self.emitted = 0;
        self.observation_start = 0;
        self.skip_until = 0;
        self.reports = 0;
        self.pending = None;
    }
    /// Start observation late without resetting the generation's emitted clock.
    /// Omit an incomplete leading window; all reported full windows remain on
    /// the same global 2,400-frame boundaries. This does not affect audio.
    pub fn start_at(&mut self, emitted: u64) -> Result<(), MeterWindowError> {
        let remainder = emitted % WINDOW_FRAMES;
        let skip_until = if remainder == 0 {
            emitted
        } else {
            emitted
                .checked_add(WINDOW_FRAMES - remainder)
                .ok_or(MeterWindowError::CounterOverflow)?
        };
        self.emitted = emitted;
        self.observation_start = emitted;
        self.skip_until = skip_until;
        self.reports = skip_until / WINDOW_FRAMES;
        self.pending = None;
        Ok(())
    }
    pub fn push_block(
        &mut self,
        block: &MeterBlock,
        mix_revision: u64,
        mut emit: impl FnMut(MeterReport),
    ) -> Result<(), MeterWindowError> {
        block
            .song_start()
            .checked_add(block.frames() as u64)
            .ok_or(MeterWindowError::CounterOverflow)?;
        self.emitted
            .checked_add(block.frames() as u64)
            .ok_or(MeterWindowError::CounterOverflow)?;
        for frame in 0..block.frames() {
            let master = block
                .master_sample(frame)
                .ok_or(MeterWindowError::InvalidBlock)?;
            if !master.iter().all(|sample| sample.is_finite()) {
                return Err(MeterWindowError::InvalidBlock);
            }
            let tracks = std::array::from_fn(|slot| {
                if slot < self.track_count {
                    block.track_sample(slot, frame)
                } else {
                    None
                }
            });
            if let Some(report) = self.push_frame(
                block.song_start() + frame as u64,
                master,
                &tracks,
                mix_revision,
            )? {
                emit(report);
            }
        }
        Ok(())
    }
    fn push_frame(
        &mut self,
        source: u64,
        master: [f32; 2],
        tracks: &[Option<[f32; 2]>; MAX_TRACKS],
        mix_revision: u64,
    ) -> Result<Option<MeterReport>, MeterWindowError> {
        let next = self
            .emitted
            .checked_add(1)
            .ok_or(MeterWindowError::CounterOverflow)?;
        if self.emitted >= self.skip_until {
            let source_end = source
                .checked_add(1)
                .ok_or(MeterWindowError::CounterOverflow)?;
            if self.pending.is_none() {
                self.pending = Some(MeterReport {
                    report_index: 0,
                    observation_start_emitted: self.observation_start,
                    emitted_start: self.emitted,
                    emitted_end_exclusive: self.emitted,
                    first_song_frame: source,
                    last_song_frame_end_exclusive: source,
                    wrap_count: 0,
                    first_mix_revision: mix_revision,
                    last_mix_revision: mix_revision,
                    master_peak: [0.0; 2],
                    partial: false,
                    track_count: self.track_count,
                    track_peaks: std::array::from_fn(|slot| {
                        self.available[slot].then_some([0.0; 2])
                    }),
                });
            }
            let pending = self.pending.as_mut().unwrap();
            if pending.frames() != 0 && source > pending.last_song_frame_end_exclusive {
                // A caller must reset after a manual seek; no silent forward
                // gap may masquerade as a continuously observed Song interval.
                return Err(MeterWindowError::InvalidBlock);
            }
            if pending.frames() != 0 && source < pending.last_song_frame_end_exclusive {
                pending.wrap_count = pending
                    .wrap_count
                    .checked_add(1)
                    .ok_or(MeterWindowError::CounterOverflow)?;
            }
            pending.emitted_end_exclusive = next;
            pending.last_song_frame_end_exclusive = source_end;
            pending.last_mix_revision = mix_revision;
            for (peak, sample) in pending.master_peak.iter_mut().zip(master) {
                *peak = peak.max(sample.abs());
            }
            for (peak, sample) in pending.track_peaks[..self.track_count]
                .iter_mut()
                .zip(tracks)
            {
                match (peak.as_mut(), sample) {
                    (Some(peak), Some(sample)) if sample.iter().all(|s| s.is_finite()) => {
                        for (peak, sample) in peak.iter_mut().zip(sample) {
                            *peak = peak.max(sample.abs());
                        }
                    }
                    _ => *peak = None,
                }
            }
        }
        self.emitted = next;
        if next.is_multiple_of(WINDOW_FRAMES) {
            self.take_report()
        } else {
            Ok(None)
        }
    }
    fn take_report(&mut self) -> Result<Option<MeterReport>, MeterWindowError> {
        let Some(mut report) = self.pending.take() else {
            return Ok(None);
        };
        self.reports = self
            .reports
            .checked_add(1)
            .ok_or(MeterWindowError::CounterOverflow)?;
        report.report_index = self.reports;
        report.partial = report.frames() != WINDOW_FRAMES;
        Ok(Some(report))
    }
    /// Explicit natural end only; Stop, error and seek discard instead. A short
    /// final report carries its actual frame count and partial=true, never padding.
    pub fn finish_partial(
        &mut self,
        mut emit: impl FnMut(MeterReport),
    ) -> Result<(), MeterWindowError> {
        if let Some(report) = self.take_report()? {
            emit(report);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        meters::TrackMeterRow,
        song::{InstrumentId, TrackId},
    };

    fn layout(shared: bool) -> MeterLayout {
        MeterLayout {
            tracks: vec![
                TrackMeterRow {
                    track: TrackId(1),
                    availability: TrackMeterAvailability::Available,
                },
                TrackMeterRow {
                    track: TrackId(2),
                    availability: if shared {
                        TrackMeterAvailability::SharedLegacy {
                            instruments: vec![InstrumentId(1)],
                        }
                    } else {
                        TrackMeterAvailability::Available
                    },
                },
            ],
            tracks_unavailable: None,
        }
    }
    fn frame(
        windows: &mut MeterWindows,
        song: u64,
        value: f32,
        revision: u64,
    ) -> Option<MeterReport> {
        let mut tracks = [None; MAX_TRACKS];
        tracks[0] = Some([value, -value / 2.0]);
        tracks[1] = Some([-value / 4.0, value / 8.0]);
        windows
            .push_frame(song, [value / 2.0, -value / 4.0], &tracks, revision)
            .unwrap()
    }
    #[test]
    fn meter_windows_have_exact_global_boundaries_and_unrounded_raw_peaks() {
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        let mut reports = Vec::new();
        for index in 0..4800 {
            if let Some(report) = frame(
                &mut windows,
                700 + index,
                (index % 17) as f32 / 32.0,
                index / 1000,
            ) {
                reports.push(report);
            }
        }
        assert_eq!(reports.len(), 2);
        for (index, report) in reports.iter().enumerate() {
            assert_eq!(report.report_index, index as u64 + 1);
            assert_eq!(report.emitted_start, index as u64 * 2400);
            assert_eq!(report.emitted_end_exclusive, (index as u64 + 1) * 2400);
            assert_eq!(report.first_song_frame, 700 + report.emitted_start);
            assert_eq!(
                report.last_song_frame_end_exclusive,
                700 + report.emitted_end_exclusive
            );
            assert_eq!(report.frames(), 2400);
            assert!(!report.partial);
            assert_eq!(report.master_peak, [0.25, 0.125]);
            assert_eq!(
                report.track_peaks(),
                &[Some([0.5, 0.25]), Some([0.125, 0.0625])]
            );
        }
        assert_eq!(
            (reports[0].first_mix_revision, reports[0].last_mix_revision),
            (0, 2)
        );
        assert_eq!(
            (reports[1].first_mix_revision, reports[1].last_mix_revision),
            (2, 4)
        );
    }
    #[test]
    fn meter_loops_count_only_wraps_inside_each_window() {
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        let mut reports = Vec::new();
        for emitted in 0..4800 {
            if let Some(report) = frame(&mut windows, emitted % 600, 0.25, 0) {
                reports.push(report);
            }
        }
        assert_eq!(reports.len(), 2);
        for report in reports {
            assert_eq!(report.wrap_count, 3); // boundary wrap excluded
            assert_eq!(report.first_song_frame, 0);
            assert_eq!(report.last_song_frame_end_exclusive, 600);
            assert_eq!(report.frames(), 2400);
        }
    }
    #[test]
    fn meter_partial_is_explicit_only_and_reset_discards_it() {
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        for i in 0..2399 {
            assert!(frame(&mut windows, i, 0.5, 0).is_none());
        }
        let mut reports = Vec::new();
        windows
            .finish_partial(|report| reports.push(report))
            .unwrap();
        assert_eq!(reports.len(), 1);
        assert!(reports[0].partial);
        assert_eq!(reports[0].frames(), 2399);
        windows
            .finish_partial(|_| panic!("duplicate partial"))
            .unwrap();
        windows.reset();
        for i in 0..99 {
            assert!(frame(&mut windows, i, 0.75, 0).is_none());
        }
        windows.reset();
        let mut report = None;
        for i in 0..2400 {
            report = frame(&mut windows, 500 + i, 0.25, 9).or(report);
        }
        let report = report.unwrap();
        assert_eq!(
            (
                report.report_index,
                report.emitted_start,
                report.first_song_frame
            ),
            (1, 0, 500)
        );
        assert_eq!(report.track_peaks()[0], Some([0.25, 0.125]));
    }
    #[test]
    fn meter_nonfinite_is_sticky_for_one_whole_row_window_not_audio_or_other_rows() {
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        let mut tracks = [None; MAX_TRACKS];
        tracks[0] = Some([0.5, 0.25]);
        tracks[1] = Some([f32::INFINITY, 0.0]);
        assert!(windows
            .push_frame(0, [0.25, 0.125], &tracks, 0)
            .unwrap()
            .is_none());
        let mut report = None;
        for i in 1..2400 {
            report = frame(&mut windows, i, 0.5, 0).or(report);
        }
        assert_eq!(report.unwrap().track_peaks(), &[Some([0.5, 0.25]), None]);
        for i in 2400..4800 {
            report = frame(&mut windows, i, 0.5, 0).or(report);
        }
        assert_eq!(
            report.unwrap().track_peaks(),
            &[Some([0.5, 0.25]), Some([0.125, 0.0625])]
        );
    }
    #[test]
    fn meter_shared_legacy_and_master_only_layouts_never_fabricate_zero_totals() {
        let mut windows = MeterWindows::new(&layout(true)).unwrap();
        for i in 0..2399 {
            frame(&mut windows, i, 0.5, 0);
        }
        let report = frame(&mut windows, 2399, 0.5, 0).unwrap();
        assert_eq!(report.track_peaks(), &[Some([0.5, 0.25]), None]);
        let master_only = MeterLayout {
            tracks: vec![],
            tracks_unavailable: Some("unsupported".into()),
        };
        let mut windows = MeterWindows::new(&master_only).unwrap();
        for i in 0..2399 {
            frame(&mut windows, i, 0.5, 0);
        }
        let report = frame(&mut windows, 2399, 0.5, 0).unwrap();
        assert!(report.track_peaks().is_empty());
        assert_eq!(report.master_peak, [0.25, 0.125]);
    }
    #[test]
    fn meter_checked_counter_failures_never_wrap_or_make_a_report() {
        let tracks = [Some([0.1; 2]); MAX_TRACKS];
        for kind in 0..4 {
            let mut windows = MeterWindows::new(&layout(false)).unwrap();
            let source = if kind == 1 { u64::MAX } else { 0 };
            match kind {
                0 => windows.emitted = u64::MAX,
                2 => {
                    frame(&mut windows, 1, 0.1, 0);
                    windows.pending.as_mut().unwrap().wrap_count = u64::MAX;
                }
                3 => {
                    windows.emitted = 2399;
                    windows.reports = u64::MAX;
                }
                _ => {}
            }
            assert_eq!(
                windows.push_frame(source, [0.1; 2], &tracks, 0),
                Err(MeterWindowError::CounterOverflow)
            );
        }
    }
    #[test]
    fn meter_forward_gap_requires_explicit_reset_instead_of_fake_continuity() {
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        frame(&mut windows, 100, 0.25, 0);
        let tracks = [Some([0.25; 2]); MAX_TRACKS];
        assert_eq!(
            windows.push_frame(102, [0.25; 2], &tracks, 0),
            Err(MeterWindowError::InvalidBlock)
        );
        windows.reset();
        assert!(frame(&mut windows, 102, 0.25, 0).is_none());
        windows
            .finish_partial(|report| {
                assert_eq!(report.first_song_frame, 102);
                assert_eq!(report.frames(), 1);
            })
            .unwrap();
    }
    #[test]
    fn meter_late_start_preserves_alignment_and_explicit_origin() {
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        windows.start_at(2048).unwrap();
        for emitted in 2048..4799 {
            assert!(frame(&mut windows, emitted % 600, 0.25, 0).is_none());
        }
        let report = frame(&mut windows, 4799 % 600, 0.25, 0).unwrap();
        assert_eq!(report.observation_start_emitted, 2048);
        assert_eq!(
            (
                report.report_index,
                report.emitted_start,
                report.emitted_end_exclusive
            ),
            (2, 2400, 4800)
        );
        assert_eq!(report.wrap_count, 3);
        assert!(!report.partial);
        assert_eq!(
            windows.start_at(u64::MAX),
            Err(MeterWindowError::CounterOverflow)
        );
        windows.reset();
        frame(&mut windows, 0, 0.25, 0);
        windows
            .finish_partial(|report| assert_eq!(report.observation_start_emitted, 0))
            .unwrap();
    }
    #[test]
    fn meter_oversized_public_layout_refuses_and_serialization_is_bounded() {
        let mut bad = layout(false);
        bad.tracks = vec![bad.tracks[0].clone(); 65];
        assert!(matches!(
            MeterWindows::new(&bad),
            Err(MeterWindowError::TooManyTracks)
        ));
        let mut windows = MeterWindows::new(&layout(false)).unwrap();
        frame(&mut windows, 10, 0.5, 0);
        let mut report = None;
        windows
            .finish_partial(|value| report = Some(value))
            .unwrap();
        let json = serde_json::to_value(report.unwrap()).unwrap();
        assert_eq!(json["track_peaks"].as_array().unwrap().len(), 2);
        assert_eq!(json["first_song_frame"], 10);
        assert_eq!(json["frames"], 1);
        assert!(json.get("first_source_frame").is_none());
    }
}
