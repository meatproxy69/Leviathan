//! Render an existing editable project without changing it.
fn main() {
    let result = (|| -> Result<(), String> {
        let args: Vec<_> = std::env::args_os().skip(1).collect();
        if args.len() != 2 {
            return Err("usage: render_song PROJECT.json NEW_OUTPUT.wav".into());
        }
        let (song, _) = aqua_pattern_core::project::load(std::path::Path::new(&args[0]))?;
        let (stats, _) =
            aqua_pattern_core::song_render::export(&song, None, std::path::Path::new(&args[1]))?;
        println!("{} frames; peak {:.6}", stats.frames, stats.peak);
        Ok(())
    })();
    if let Err(error) = result {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
