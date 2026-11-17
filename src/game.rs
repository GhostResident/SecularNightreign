use std::{
    fs::{self, File, Metadata},
    io::{Read, Seek, SeekFrom},
    os::windows::fs::MetadataExt,
    path::{Path, PathBuf},
    sync::LazyLock,
};

use anyhow::{Context, Result, ensure};
use regex::Regex;
use serde::Serialize;
use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_REPARSE_POINT;

use crate::platform;

pub const CHAIN: [&str; 4] = ["1.3.0", "1.3.1", "1.3.2", "1.3.3"];

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub root: PathBuf,
    pub game_dir: PathBuf,
    pub raw_version: Option<String>,
    pub version: Option<String>,
    pub supported: bool,
    pub exe_bytes: u64,
}

pub fn inspect(path: &Path) -> Result<Game> {
    ensure!(!path.as_os_str().is_empty(), "Укажите папку игры");
    let selected = dunce::canonicalize(path)
        .with_context(|| format!("Папка не найдена: {}", path.display()))?;
    ordinary_dir(&selected)?;
    let nested = selected.join("Game");
    let (root, game_dir) = if selected
        .file_name()
        .is_some_and(|name| name.eq_ignore_ascii_case("Game"))
        && selected.join("nightreign.exe").is_file()
    {
        let root = selected.parent().unwrap_or(&selected).to_owned();
        (root, selected)
    } else if nested.exists() {
        ordinary_dir(&nested)?;
        (selected, nested)
    } else {
        (selected.clone(), selected)
    };
    let exe = game_dir.join("nightreign.exe");
    let exe_bytes = ordinary_file(&exe)
        .with_context(|| format!("Не найден nightreign.exe в {}", game_dir.display()))?
        .len();
    let raw_version = platform::file_version(&exe);
    let version = raw_version.as_deref().and_then(normalize);
    let supported = version.as_deref().is_some_and(|v| CHAIN.contains(&v));
    Ok(Game {
        root,
        game_dir,
        raw_version,
        version,
        supported,
        exe_bytes,
    })
}

pub fn normalize(raw: &str) -> Option<String> {
    static VERSION: LazyLock<Regex> =
        LazyLock::new(|| Regex::new(r"^(\d+)[.,](\d+)(?:[.,](\d+))?").unwrap());
    let compact: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    let fields = VERSION.captures(&compact)?;
    let major = fields[1].parse::<u32>().ok()?;
    let minor = fields[2].parse::<u32>().ok()?;
    let patch = fields
        .get(3)
        .map_or(Some(0), |value| value.as_str().parse().ok())?;
    Some(format!("{major}.{minor}.{patch}"))
}

pub fn is_reparse(metadata: &Metadata) -> bool {
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

pub fn ordinary_dir(path: &Path) -> Result<()> {
    let metadata = fs::symlink_metadata(path).with_context(|| path.display().to_string())?;
    ensure!(
        metadata.is_dir() && !is_reparse(&metadata),
        "Требуется обычная папка: {}",
        path.display()
    );
    Ok(())
}

pub fn ordinary_file(path: &Path) -> Result<Metadata> {
    let metadata = fs::symlink_metadata(path).with_context(|| path.display().to_string())?;
    ensure!(
        metadata.is_file() && !is_reparse(&metadata),
        "Требуется обычный файл: {}",
        path.display()
    );
    Ok(metadata)
}

pub fn check_pe(path: &Path) -> Result<()> {
    let metadata = ordinary_file(path)?;
    let mut file = File::open(path)?;
    let mut header = [0_u8; 64];
    let message = || format!("Файл не является Windows EXE: {}", path.display());
    file.read_exact(&mut header).with_context(message)?;
    ensure!(&header[..2] == b"MZ", "{}", message());
    let pe_offset = u32::from_le_bytes(header[60..64].try_into().unwrap()) as u64;
    ensure!(
        pe_offset >= 64 && pe_offset + 4 <= metadata.len(),
        "{}",
        message()
    );
    file.seek(SeekFrom::Start(pe_offset))?;
    let mut signature = [0; 4];
    file.read_exact(&mut signature)?;
    ensure!(&signature == b"PE\0\0", "{}", message());
    Ok(())
}
