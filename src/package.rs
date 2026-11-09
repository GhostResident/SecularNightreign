use anyhow::{Context, Result, bail};
use std::{
    fs::{self, File},
    io,
    path::{Component, Path, PathBuf},
};
use tempfile::TempDir;

pub struct ExtractedPackage {
    _temp: TempDir,
    pub root: PathBuf,
}

pub fn extract_package(zip_path: &Path) -> Result<ExtractedPackage> {
    let temp = tempfile::Builder::new()
        .prefix("SecularNightreign_")
        .tempdir()?;
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?)
        .context("скачанный файл не является корректным ZIP-архивом")?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(safe_name) = entry.enclosed_name() else {
            bail!("опасный путь внутри ZIP: {}", entry.name());
        };
        if safe_name
            .components()
            .any(|c| matches!(c, Component::ParentDir))
        {
            bail!("опасный путь внутри ZIP: {}", entry.name());
        }
        let output = temp.path().join(safe_name);
        if entry.is_dir() {
            fs::create_dir_all(&output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(&output)?;
        io::copy(&mut entry, &mut file)?;
    }
    let root = find_root(temp.path()).context("в архиве нет version.txt и папки mod")?;
    Ok(ExtractedPackage { _temp: temp, root })
}

pub fn extract_zip_to(zip_path: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    let base = fs::canonicalize(destination).unwrap_or_else(|_| destination.to_path_buf());
    let mut archive = zip::ZipArchive::new(File::open(zip_path)?)
        .context("скачанный файл не является корректным ZIP-архивом")?;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index)?;
        let Some(name) = entry.enclosed_name() else {
            bail!("опасный путь внутри ZIP: {}", entry.name());
        };
        let output = base.join(name);
        if !output.starts_with(&base) {
            bail!("опасный путь внутри ZIP");
        }
        if entry.is_dir() {
            fs::create_dir_all(output)?;
            continue;
        }
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(output)?;
        io::copy(&mut entry, &mut file)?;
    }
    Ok(())
}

fn find_root(start: &Path) -> Option<PathBuf> {
    if start.join("version.txt").is_file() && start.join("mod").is_dir() {
        return Some(start.to_path_buf());
    }
    for entry in fs::read_dir(start).ok()?.flatten() {
        let path = entry.path();
        if path.is_dir() && path.join("version.txt").is_file() && path.join("mod").is_dir() {
            return Some(path);
        }
    }
    None
}
