use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{fs, fs::OpenOptions, io::Write, path::PathBuf};

use crate::config::APP_ID;

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct SavedState {
    pub game_dir: String,
}

pub fn app_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(std::env::temp_dir)
        .join(APP_ID)
}

pub fn downloads_dir() -> PathBuf {
    dirs::download_dir()
        .unwrap_or_else(|| app_dir().join("Downloads"))
        .join(APP_ID)
}

pub fn load() -> SavedState {
    let path = app_dir().join("state.json");
    let state: SavedState = fs::read(&path)
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default();
    if !state.game_dir.is_empty() {
        return state;
    }
    let legacy = app_dir().join("game.txt");
    fs::read_to_string(legacy)
        .ok()
        .map(|game_dir| SavedState {
            game_dir: game_dir.trim_start_matches('\u{feff}').trim().to_owned(),
        })
        .unwrap_or_default()
}

pub fn save(state: &SavedState) -> Result<()> {
    let dir = app_dir();
    fs::create_dir_all(&dir).context("не удалось создать папку настроек")?;
    fs::write(dir.join("state.json"), serde_json::to_vec_pretty(state)?)
        .context("не удалось сохранить настройки")
}

pub fn clear() {
    let _ = fs::remove_file(app_dir().join("state.json"));
}

pub fn append_log(line: &str) {
    let dir = app_dir();
    if fs::create_dir_all(&dir).is_err() {
        return;
    }
    let path = dir.join("launcher.log");
    if path.metadata().is_ok_and(|m| m.len() > 1024 * 1024) {
        let _ = fs::rename(&path, dir.join("launcher.old.log"));
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(
            file,
            "{} {}",
            chrono::Local::now().format("%Y-%m-%d %H:%M:%S"),
            line
        );
    }
}
