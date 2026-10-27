#[path = "support/transport_diagnostics.rs"]
mod diagnostics;

#[test]
fn bounded_device_free_transport_matrix_and_concurrent_lifecycle() {
    let dir =
        std::env::temp_dir().join(format!("transport-diagnostics-test-{}", std::process::id()));
    let report = diagnostics::run(&dir).unwrap();
    assert_eq!(report.parity.len(), 8);
    assert!(report
        .parity
        .iter()
        .all(|case| case.exact_pcm && case.underrun_frames == 0));
    assert!(report.normal_matrix_frames <= 1_000_000);
    assert_eq!(report.callback_memory, [0, 0, 0]);
    assert_eq!(report.concurrent.completion, "intended_stop");
    assert!(report.concurrent.commands_completed >= 16);
    assert!(report.concurrent.pump_calls <= 256);
    assert!(!report.physical_device_opened);
    // Only this test's newly created authored directory; never user media.
    std::fs::remove_dir_all(dir).unwrap();
}

#[test]
fn diagnostic_output_requires_a_new_absolute_directory() {
    assert!(diagnostics::run(std::path::Path::new("relative-diagnostic-output")).is_err());
    let dir = std::env::temp_dir().join(format!("transport-existing-test-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    std::fs::write(dir.join("keep.txt"), b"existing evidence").unwrap();
    assert!(diagnostics::run(&dir).is_err());
    assert_eq!(
        std::fs::read(dir.join("keep.txt")).unwrap(),
        b"existing evidence"
    );
    assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 1);
    std::fs::remove_dir_all(dir).unwrap();
}
