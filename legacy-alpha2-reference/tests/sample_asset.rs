use aqua_pattern_core::sample_asset::{self, MAX_SAMPLE_VALUES};
use std::path::PathBuf;

fn wav() -> (PathBuf, Vec<u8>) {
    let path = std::env::temp_dir().join(format!("leviathan-import-{}.wav", std::process::id()));
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: 48_000,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut writer = hound::WavWriter::create(&path, spec).unwrap();
    for sample in [0i16, 1000, -1000, 0] {
        writer.write_sample(sample).unwrap();
    }
    writer.finalize().unwrap();
    let bytes = std::fs::read(&path).unwrap();
    (path, bytes)
}

#[test]
fn import_hashes_original_bytes_decodes_native_frames_and_detects_replacement() {
    let (path, encoded) = wav();
    let imported = sample_asset::import(&path).unwrap();
    assert_eq!(
        imported.asset.sha256,
        sample_asset::content_sha256(&encoded)
    );
    assert_eq!(imported.asset.sample_rate, 48_000);
    assert_eq!(imported.asset.channels, 1);
    assert_eq!(imported.asset.frames, 4);
    sample_asset::verify_identity(&imported.asset, &path).unwrap();
    std::fs::write(&path, b"replacement bytes").unwrap();
    assert!(sample_asset::verify_identity(&imported.asset, &path).is_err());
    assert!(MAX_SAMPLE_VALUES > imported.asset.frames as usize);
    std::fs::remove_file(path).unwrap();
}
