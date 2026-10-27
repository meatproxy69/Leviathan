//! Bounded, strict WAV/FLAC import on the control/render worker. Never called by
//! the audio callback. Original encoded bytes are identified, not modified.
mod sha256;
use crate::song::SampleAsset;
use phonic::AudioFileBuffer;
use std::{
    fs::File,
    io::{Cursor, Read},
    path::Path,
    sync::Arc,
};
use symphonia::core::{
    audio::SampleBuffer, codecs::DecoderOptions, errors::Error, formats::FormatOptions,
    io::MediaSourceStream, meta::MetadataOptions, probe::Hint,
};
pub const MAX_ENCODED_BYTES: usize = 64 * 1024 * 1024;
pub const MAX_SAMPLE_VALUES: usize = 16 * 1024 * 1024;
pub const MAX_TOTAL_ENCODED_BYTES: u64 = 256 * 1024 * 1024;
#[cfg(test)]
thread_local! { static METRICS: std::cell::Cell<[usize; 3]> = const { std::cell::Cell::new([0; 3]) }; }
#[cfg(test)]
pub(crate) fn reset_metrics() {
    METRICS.with(|metrics| metrics.set([0; 3]));
}
#[cfg(test)]
pub(crate) fn metrics() -> [usize; 3] {
    METRICS.with(|metrics| metrics.get())
}
#[cfg(test)]
fn count(index: usize) {
    METRICS.with(|metrics| {
        let mut counts = metrics.get();
        counts[index] += 1;
        metrics.set(counts);
    });
}
pub(crate) fn content_sha256(bytes: &[u8]) -> String {
    sha256::hex(bytes)
}
fn fingerprint(bytes: &[u8]) -> String {
    #[cfg(test)]
    count(1);
    sha256::hex(bytes)
}
const MAX_PACKET_FRAMES: u64 = 65536;

pub struct ImportedSample {
    pub asset: SampleAsset,
    /// Native-rate interleaved PCM plus one all-zero guard frame. For analysis,
    /// use only `asset.frames * asset.channels` values, not the guard.
    pub buffer: Arc<AudioFileBuffer>,
}
fn read(path: &Path) -> Result<Vec<u8>, String> {
    #[cfg(test)]
    count(0);
    if !path
        .metadata()
        .map_err(|e| format!("Source {}: {e}", path.display()))?
        .is_file()
    {
        return Err("Source is not a regular file".into());
    }
    let file = File::open(path).map_err(|e| format!("Source {}: {e}", path.display()))?;
    let metadata = file.metadata().map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.len() > MAX_ENCODED_BYTES as u64 {
        return Err("Source must be a regular file at most 64 MiB encoded".into());
    }
    let mut bytes = Vec::new();
    file.take(MAX_ENCODED_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > MAX_ENCODED_BYTES {
        return Err("Source exceeds 64 MiB encoded".into());
    }
    Ok(bytes)
}
/// Caller must authorize the source path before this function accesses it.
/// Import never resamples, normalizes, writes files, or enables embedded loops.
/// Slice regions are explicit one-shots; loop-aware playback is a later policy.
pub fn import(path: &Path) -> Result<ImportedSample, String> {
    let path = path
        .canonicalize()
        .map_err(|e| format!("Source {}: {e}", path.display()))?;
    let path = crate::project::normalize_canonical_path(path);
    let bytes = read(&path)?;
    let hash = fingerprint(&bytes);
    decode(&path, bytes, MAX_SAMPLE_VALUES, hash)
}
/// Hash-only check for authorized project storage paths; does not decode audio.
pub fn verify_identity(asset: &SampleAsset, path: &Path) -> Result<(), String> {
    if fingerprint(&read(path)?) != asset.sha256 {
        return Err(format!(
            "Source {} changed: SHA-256 does not match the imported asset",
            path.display()
        ));
    }
    Ok(())
}
/// Verify immutable identity and metadata before a shared buffer is reused.
/// `remaining_values` bounds this decode's contribution to the Engine budget.
pub fn load_verified(
    asset: &SampleAsset,
    path: &Path,
    remaining_values: usize,
) -> Result<Arc<AudioFileBuffer>, String> {
    let bytes = read(path)?;
    let hash = fingerprint(&bytes);
    if hash != asset.sha256 {
        return Err(format!(
            "Source {} changed: SHA-256 does not match the imported asset",
            path.display()
        ));
    }
    let imported = decode(path, bytes, remaining_values, hash)?;
    if imported.asset.sample_rate != asset.sample_rate
        || imported.asset.channels != asset.channels
        || imported.asset.frames != asset.frames
    {
        return Err("Source decoded metadata does not match the imported asset".into());
    }
    Ok(imported.buffer)
}
fn decode(
    path: &Path,
    bytes: Vec<u8>,
    remaining_values: usize,
    sha256: String,
) -> Result<ImportedSample, String> {
    #[cfg(test)]
    count(2);
    let is_wav = bytes.starts_with(b"RIFF") && bytes.get(8..12) == Some(b"WAVE");
    let is_flac = bytes.starts_with(b"fLaC");
    if !is_wav && !is_flac {
        return Err("Source import supports WAV and FLAC only".into());
    }
    let stream = MediaSourceStream::new(Box::new(Cursor::new(bytes)), Default::default());
    let probe = symphonia::default::get_probe()
        .format(
            &Hint::new(),
            stream,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )
        .map_err(|e| format!("Source probe: {e}"))?;
    let mut format = probe.format;
    if format.tracks().len() != 1 {
        return Err("Source must contain exactly one audio track".into());
    }
    let track = format.default_track().ok_or("Source has no audio track")?;
    let params = &track.codec_params;
    let rate = params
        .sample_rate
        .ok_or("Source has no declared sample rate")?;
    let channels = params
        .channels
        .ok_or("Source has no declared channels")?
        .count();
    let frames = params
        .n_frames
        .ok_or("Source must declare its decoded frame count")?;
    let values = frames
        .checked_add(1)
        .and_then(|n| n.checked_mul(channels as u64))
        .ok_or("Source decoded size overflow")?;
    if !(1..=384000).contains(&rate)
        || !(1..=2).contains(&channels)
        || frames == 0
        || values > MAX_SAMPLE_VALUES.min(remaining_values) as u64
    {
        return Err("Source requires nonempty mono/stereo, rate 1..=384000, and total decoded PCM at most 64 MiB".into());
    }
    if params
        .max_frames_per_packet
        .is_some_and(|n| n > MAX_PACKET_FRAMES)
    {
        return Err("Source packet exceeds 65536 frames".into());
    }
    let track_id = track.id;
    let mut decoder = symphonia::default::get_codecs()
        .make(params, &DecoderOptions { verify: true })
        .map_err(|e| format!("Source decoder: {e}"))?;
    let mut pcm = Vec::with_capacity(values as usize);
    loop {
        let packet = match format.next_packet() {
            Ok(packet) => packet,
            Err(Error::IoError(e)) if e.kind() == std::io::ErrorKind::UnexpectedEof => break,
            Err(e) => return Err(format!("Source packet: {e}")),
        };
        if packet.track_id() != track_id {
            return Err("Unexpected source track".into());
        }
        if packet.dur() > MAX_PACKET_FRAMES {
            return Err("Source packet exceeds 65536 frames".into());
        }
        let decoded = decoder
            .decode(&packet)
            .map_err(|e| format!("Corrupt source packet: {e}"))?;
        if decoded.spec().rate != rate
            || decoded.spec().channels.count() != channels
            || decoded.capacity() as u64 > MAX_PACKET_FRAMES
        {
            return Err("Source format changed or packet decoded capacity exceeds limit".into());
        }
        let count = decoded
            .frames()
            .checked_mul(channels)
            .ok_or("Decoded packet size overflow")?;
        if pcm
            .len()
            .checked_add(count)
            .is_none_or(|n| n as u64 > frames * channels as u64)
        {
            return Err("Source exceeds declared decoded frame count".into());
        }
        let mut samples = SampleBuffer::<f32>::new(decoded.capacity() as u64, *decoded.spec());
        samples.copy_interleaved_ref(decoded);
        if samples.samples().iter().any(|sample| !sample.is_finite()) {
            return Err("Source contains non-finite PCM".into());
        }
        pcm.extend_from_slice(samples.samples());
    }
    if pcm.len() as u64 != frames * channels as u64 {
        return Err("Truncated source: decoded frame count differs from its declaration".into());
    }
    if decoder.finalize().verify_ok == Some(false) {
        return Err("Source decoder integrity verification failed".into());
    }
    pcm.extend(std::iter::repeat_n(0.0, channels));
    let buffer = AudioFileBuffer::new(pcm, channels, rate, None).map_err(|e| e.to_string())?;
    Ok(ImportedSample {
        asset: SampleAsset {
            path: path.to_owned(),
            sha256,
            sample_rate: rate,
            channels: channels as u16,
            frames,
        },
        buffer: Arc::new(buffer),
    })
}
