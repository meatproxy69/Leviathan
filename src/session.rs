//! Destructive document transitions are staged until save/discard/cancel resolves.
use crate::{
    editor::Editor,
    project::{self, SaveMode},
    song::new_song,
};
use std::path::PathBuf;
#[derive(Debug, Clone)]
pub enum Action {
    New,
    Open(PathBuf),
    Close,
}
#[derive(Debug)]
pub struct Overwrite {
    pub path: PathBuf,
    bytes: Vec<u8>,
    resume: bool,
}
#[derive(Debug, Default)]
pub struct Session {
    pub path: Option<PathBuf>,
    pub pending: Option<Action>,
    pub overwrite: Option<Overwrite>,
    pub close_ready: bool,
    pub notice: String,
}
impl Session {
    pub fn request(&mut self, editor: &mut Editor, action: Action) -> Result<(), String> {
        if self.close_ready {
            return Err("Project is closing".into());
        }
        if self.pending.is_some() || self.overwrite.is_some() {
            return Err("Resolve the current project confirmation first".into());
        }
        // A staged Open must keep the same target if a directory alias or CWD changes.
        let action = match action {
            Action::Open(path) => Action::Open(project::absolute(&path)?),
            action => action,
        };
        if editor.dirty {
            self.pending = Some(action);
            Ok(())
        } else {
            self.execute(editor, action)
        }
    }
    fn execute(&mut self, editor: &mut Editor, action: Action) -> Result<(), String> {
        match action {
            Action::New => {
                let song = new_song();
                editor.replace_document(song)?;
                self.path = None;
                self.notice = "New song".into();
            }
            Action::Open(path) => {
                let (song, path) = project::load(&path)?;
                editor.replace_document(song)?;
                self.path = Some(path);
                self.notice = "Project opened".into();
            }
            Action::Close => self.close_ready = true,
        }
        Ok(())
    }
    pub fn cancel(&mut self) {
        self.pending = None;
        self.overwrite = None;
    }
    /// Failed load keeps both the current document and the pending decision intact.
    pub fn discard(&mut self, editor: &mut Editor) -> Result<(), String> {
        if self.overwrite.is_some() {
            return Err("Resolve the overwrite confirmation first".into());
        }
        if let Some(action) = self.pending.clone() {
            self.execute(editor, action)?;
            self.pending = None;
        }
        Ok(())
    }
    pub fn save_request(
        &mut self,
        editor: &mut Editor,
        path: PathBuf,
        resume: bool,
    ) -> Result<(), String> {
        if self.close_ready {
            return Err("Project is closing".into());
        }
        if self.overwrite.is_some() || (self.pending.is_some() && !resume) {
            return Err("Resolve the current project confirmation first".into());
        }
        // Capture the canonical parent, not just CWD, before asking for confirmation.
        let path = project::absolute(&path)?;
        match std::fs::symlink_metadata(&path) {
            Ok(_) => {
                self.overwrite = Some(Overwrite {
                    bytes: project::existing_bytes(&path)?,
                    path,
                    resume,
                });
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.save(editor, path, SaveMode::CreateNew, resume)
            }
            Err(e) => Err(e.to_string()),
        }
    }
    fn save(
        &mut self,
        editor: &mut Editor,
        path: PathBuf,
        mode: SaveMode,
        resume: bool,
    ) -> Result<(), String> {
        let (path, warning) = project::save(&editor.song, &path, mode)?;
        self.path = Some(path);
        editor.mark_saved();
        self.notice = warning.unwrap_or_else(|| "Project saved".into());
        if resume {
            self.discard(editor)?;
        }
        Ok(())
    }
    pub fn confirm_overwrite(&mut self, editor: &mut Editor) -> Result<(), String> {
        if let Some(overwrite) = self.overwrite.take() {
            self.save(
                editor,
                overwrite.path,
                SaveMode::ReplaceConfirmed(overwrite.bytes),
                overwrite.resume,
            )?;
        }
        Ok(())
    }
}
