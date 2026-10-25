use aqua_pattern_core::{project, transport::host::Transport};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
fn run() -> Result<(), String> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: transport_probe PROJECT")?;
    let (song, _) = project::load(std::path::Path::new(&path))?;
    let mut player = Transport::default();
    for pass in 0..2 {
        player.start(song.clone(), None)?;
        let start = Instant::now();
        let mut transitioned = false;
        loop {
            player.poll();
            if let Some(e) = &player.error {
                return Err(e.clone());
            }
            let state = player.shared.as_ref().unwrap();
            if state.audible_frames.load(Ordering::Relaxed) > 12000 && !transitioned {
                state.seek(24000);
                let rows: u64 = song
                    .order
                    .iter()
                    .map(|o| song.patterns[&o.pattern].rows as u64)
                    .sum();
                let end = (rows as f64 * 48000.0 * 60.0 / (song.bpm * song.lines_per_beat as f64))
                    .round() as u64;
                state.set_loop(true, end);
                transitioned = true;
            }
            if start.elapsed() > Duration::from_millis(1600) {
                player.stop();
            }
            if !player.busy() {
                break;
            }
            if start.elapsed() > Duration::from_secs(5) {
                return Err("Transport stop/start timed out".into());
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let state = player.shared.as_ref().unwrap();
        let callbacks = state.callbacks.load(Ordering::Relaxed);
        let audible = state.audible_frames.load(Ordering::Relaxed);
        if callbacks == 0 || audible == 0 {
            return Err("No nonzero PCM reached device callback".into());
        }
        println!("pass {pass}: callbacks={callbacks}, nonzero_frames={audible}, underrun_frames={}, clean stop; hardware audibility/latency not measured",state.underruns.load(Ordering::Relaxed));
    }
    Ok(())
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
