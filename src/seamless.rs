use anyhow::{Context, Result, bail};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

use crate::config::SEAMLESS_EXE;

pub fn install_if_present(game: &Path, mut log: impl FnMut(String)) -> Result<()> {
    let current = std::env::current_exe()?;
    let Some(parent) = current.parent() else {
        return Ok(());
    };
    let archive_path = parent.join("Seamless.zip");
    if !archive_path.is_file() {
        return Ok(());
    }
    log(format!(
        "Найден {}, установка Seamless Co-op…",
        archive_path.display()
    ));
    let temp = tempfile::Builder::new()
        .prefix("Secular_Seamless_")
        .tempdir()?;
    extract_safe(&archive_path, temp.path())?;
    let launcher = find_file(temp.path(), SEAMLESS_EXE, 0)
        .with_context(|| format!("в Seamless.zip нет {SEAMLESS_EXE}"))?;
    let root = launcher
        .parent()
        .context("неверная структура Seamless.zip")?;
    let mut copied = 0_u32;
    copy_tree(root, root, game, &mut copied)?;
    log(format!(
        "Seamless Co-op установлен: {copied} файлов (существующие INI сохранены)"
    ));
    Ok(())
}

fn extract_safe(zip_path: &Path, destination: &Path) -> Result<()> {
    let mut archive = zip::ZipArchive::new(fs::File::open(zip_path)?)?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(name) = entry.enclosed_name() else {
            bail!("опасный путь в Seamless.zip");
        };
        let output = destination.join(name);
        if entry.is_dir() {
            fs::create_dir_all(output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = fs::File::create(output)?;
        io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

fn find_file(dir: &Path, name: &str, depth: u8) -> Option<PathBuf> {
    if depth > 6 {
        return None;
    }
    for entry in fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.is_file()
            && path
                .file_name()
                .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case(name))
        {
            return Some(path);
        }
        if path.is_dir()
            && let Some(found) = find_file(&path, name, depth + 1)
        {
            return Some(found);
        }
    }
    None
}

fn copy_tree(root: &Path, current: &Path, game: &Path, copied: &mut u32) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let source = entry.path();
        if source.is_dir() {
            copy_tree(root, &source, game, copied)?;
            continue;
        }
        let relative = source.strip_prefix(root)?;
        let destination = game.join(relative);
        if source
            .extension()
            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("ini"))
            && destination.is_file()
        {
            continue;
        }
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(source, destination)?;
        *copied += 1;
    }
    Ok(())
}
