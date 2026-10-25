//! UI-independent, single-job background controller. No audio work on the UI thread.
use crate::{pattern_manifest, render, RenderStats};
use std::{
    fs::{self, OpenOptions},
    path::PathBuf,
    sync::mpsc::{self, Receiver, TryRecvError},
    thread,
};
#[derive(Debug, Clone)]
pub struct SampleInfo {
    pub path: PathBuf,
    pub frames: usize,
    pub rate: u32,
    pub channels: usize,
}
#[derive(Debug)]
pub enum Request {
    Load(PathBuf),
    SongExport {
        song: crate::song::Song,
        project_dir: Option<PathBuf>,
        output: PathBuf,
    },
    Export {
        sample: PathBuf,
        output: PathBuf,
        bpm: f64,
        bars: u32,
    },
}
#[derive(Debug)]
pub enum Completed {
    Loaded(SampleInfo),
    Exported { path: PathBuf, stats: RenderStats },
}
#[derive(Default)]
pub struct Jobs {
    pending: Option<Receiver<Result<Completed, String>>>,
}
impl Jobs {
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }
    pub fn start(&mut self, request: Request) -> Result<(), String> {
        if self.busy() {
            return Err("A job is already running".into());
        }
        let (tx, rx) = mpsc::channel();
        thread::Builder::new()
            .name("leviathan-worker".into())
            .spawn(move || {
                let result =
                    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| execute(request)))
                        .unwrap_or_else(|_| {
                            Err("Audio worker panicked; inspect input and retry".into())
                        });
                let _ = tx.send(result);
            })
            .map_err(|e| e.to_string())?;
        self.pending = Some(rx);
        Ok(())
    }
    pub fn poll(&mut self) -> Option<Result<Completed, String>> {
        let result = match self.pending.as_ref()?.try_recv() {
            Ok(value) => Some(value),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => Some(Err("Audio worker disconnected".into())),
        };
        if result.is_some() {
            self.pending = None;
        }
        result
    }
}
fn execute(request: Request) -> Result<Completed, String> {
    match request {
        Request::SongExport {
            song,
            project_dir,
            output,
        } => {
            let (stats, _) = crate::song_render::export(&song, project_dir.as_deref(), &output)?;
            Ok(Completed::Exported {
                path: output,
                stats,
            })
        }
        Request::Load(path) => {
            let buffer = phonic::AudioFileBuffer::from_file(&path).map_err(|e| e.to_string())?;
            if buffer.frame_count() == 0 {
                return Err("Sample is empty".into());
            }
            Ok(Completed::Loaded(SampleInfo {
                path: path.clone(),
                frames: phonic::AudioFileInfo::from_file(&path)
                    .map_err(|e| e.to_string())?
                    .frame_count(),
                rate: buffer.sample_rate(),
                channels: buffer.channel_count(),
            }))
        }
        Request::Export {
            sample,
            output,
            bpm,
            bars,
        } => {
            if !output
                .extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("wav"))
            {
                return Err("Export filename must end in .wav".into());
            }
            let manifest = pattern_manifest(bpm, bars).map_err(|e| e.to_string())?;
            if output.exists() {
                return Err("Output already exists; choose a new filename".into());
            }
            let json = output.with_extension("events.json");
            let file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&json)
                .map_err(|e| format!("Cannot reserve manifest: {e}"))?;
            let stats = match render(&sample, &output, &manifest, 512) {
                Ok(stats) => stats,
                Err(e) => {
                    drop(file);
                    let _ = fs::remove_file(&json);
                    return Err(format!(
                        "Export failed: {e}. A partial WAV may remain at {}",
                        output.display()
                    ));
                }
            };
            serde_json::to_writer_pretty(file, &manifest)
                .map_err(|e| format!("WAV completed, but manifest write failed: {e}"))?;
            Ok(Completed::Exported {
                path: output,
                stats,
            })
        }
    }
}
