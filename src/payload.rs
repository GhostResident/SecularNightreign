use std::{
    fs::{self, File},
    io::{self, Read, Seek},
    os::windows::process::CommandExt,
    path::{Component, Path, PathBuf},
    process::{Command, Stdio},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use windows_sys::Win32::System::Threading::CREATE_NO_WINDOW;
use zip::ZipArchive;

use crate::{game, updater::WorkDir};

pub fn archive_extension(path: &Path) -> Result<&'static str> {
    let mut header = [0; 7];
    File::open(path)?.read_exact(&mut header)?;
    if header.starts_with(b"PK\x03\x04") || header.starts_with(b"PK\x05\x06") {
        Ok("zip")
    } else if &header == b"Rar!\x1a\x07\x00" || &header == b"Rar!\x1a\x07\x01" {
        Ok("rar")
    } else {
        bail!("Ожидался архив ZIP или RAR")
    }
}

pub fn extract(archive: &Path, destination: &Path) -> Result<()> {
    game::ordinary_file(archive)?;
    if archive_extension(archive)? == "zip" {
        extract_zip(File::open(archive)?, destination)?;
    } else {
        fs::create_dir(destination)?;
        use windows_sys::Win32::System::SystemInformation::GetWindowsDirectoryW;
        let mut windows = [0_u16; 260];
        let len =
            unsafe { GetWindowsDirectoryW(windows.as_mut_ptr(), windows.len() as u32) } as usize;
        ensure!(
            len > 0 && len < windows.len(),
            "Не удалось найти системный tar.exe"
        );
        let tar =
            PathBuf::from(String::from_utf16_lossy(&windows[..len])).join(r"System32\tar.exe");
        let output = Command::new(tar)
            .args(["-xf"])
            .arg(archive)
            .arg("-C")
            .arg(destination)
            // The system bsdtar also checks absolute paths, dot-dot and symlink traversal by default.
            .args(["--no-same-owner", "--no-same-permissions", "--safe-writes"])
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .context("Не удалось запустить системный tar.exe. Распакуйте RAR отдельно.")?;
        ensure!(
            output.status.success(),
            "Не удалось распаковать RAR: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        );
        // Reject reparse points in the extracted result before using any files.
        tree(destination)?;
    }
    Ok(())
}

pub fn extract_zip(reader: impl Read + Seek, destination: &Path) -> Result<()> {
    let mut zip = ZipArchive::new(reader)?;
    fs::create_dir(destination)?;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        ensure!(!entry.is_symlink(), "Ссылки в ZIP не поддерживаются");
        let path = entry.enclosed_name().context("Недопустимый путь в ZIP")?;
        safe_relative(&path)?;
        let output = destination.join(path);
        if entry.is_dir() {
            fs::create_dir_all(output)?;
        } else {
            fs::create_dir_all(output.parent().context("У файла в ZIP нет папки")?)?;
            let mut file = File::create_new(&output)
                .with_context(|| format!("Не удалось извлечь {}", output.display()))?;
            let copied = io::copy(&mut entry, &mut file)?;
            ensure!(
                copied == entry.size(),
                "Неполный файл в ZIP: {}",
                output.display()
            );
        }
    }
    Ok(())
}

fn safe_relative(path: &Path) -> Result<()> {
    ensure!(!path.as_os_str().is_empty(), "Пустое имя в патче");
    for component in path.components() {
        let Component::Normal(name) = component else {
            bail!("Недопустимый путь в патче: {}", path.display());
        };
        let text = name.to_string_lossy();
        let stem = text
            .split('.')
            .next()
            .unwrap_or_default()
            .trim_end()
            .to_ascii_uppercase();
        let reserved = matches!(
            stem.as_str(),
            "CON" | "PRN" | "AUX" | "NUL" | "CONIN$" | "CONOUT$"
        ) || ["COM", "LPT"].iter().any(|prefix| {
            stem.strip_prefix(prefix)
                .is_some_and(|tail| tail.chars().count() == 1 && "123456789¹²³".contains(tail))
        });
        ensure!(
            !reserved
                && !text.ends_with(['.', ' '])
                && !text.contains([':', '<', '>', '"', '|', '?', '*'])
                && !text.chars().any(char::is_control),
            "Недопустимое имя в патче: {}",
            path.display()
        );
    }
    Ok(())
}

pub fn find_root(path: &Path) -> Result<PathBuf> {
    game::ordinary_dir(path)?;
    let nested = path.join("Game");
    if game::ordinary_file(&path.join("nightreign.exe")).is_ok() {
        return Ok(path.to_owned());
    }
    if game::ordinary_dir(&nested).is_ok()
        && game::ordinary_file(&nested.join("nightreign.exe")).is_ok()
    {
        return Ok(nested);
    }
    let mut candidates = Vec::new();
    for entry in fs::read_dir(path)? {
        let folder = entry?.path();
        if game::ordinary_dir(&folder).is_err() {
            continue;
        }
        let nested = folder.join("Game");
        if game::ordinary_file(&folder.join("nightreign.exe")).is_ok() {
            candidates.push(folder);
        } else if game::ordinary_dir(&nested).is_ok()
            && game::ordinary_file(&nested.join("nightreign.exe")).is_ok()
        {
            candidates.push(nested);
        }
    }
    match candidates.as_slice() {
        [only] => Ok(only.clone()),
        [] => bail!("В патче не найден nightreign.exe. Укажите папку с файлами хотфикса."),
        _ => bail!("В пакете несколько копий nightreign.exe. Укажите нужную папку."),
    }
}

#[derive(Default)]
struct Tree {
    directories: Vec<PathBuf>,
    files: Vec<PathBuf>,
}

fn tree(root: &Path) -> Result<Tree> {
    fn visit(path: &Path, root: &Path, tree: &mut Tree) -> Result<()> {
        for entry in fs::read_dir(path)? {
            let full = entry?.path();
            let relative = full.strip_prefix(root)?;
            safe_relative(relative)?;
            let meta = fs::symlink_metadata(&full)?;
            ensure!(
                !game::is_reparse(&meta),
                "Ссылки в патче не поддерживаются: {}",
                relative.display()
            );
            if meta.is_dir() {
                tree.directories.push(relative.to_owned());
                visit(&full, root, tree)?;
            } else {
                ensure!(
                    meta.is_file(),
                    "Неподдерживаемый файл {}",
                    relative.display()
                );
                tree.files.push(relative.to_owned());
            }
        }
        Ok(())
    }
    game::ordinary_dir(root)?;
    let mut tree = Tree::default();
    visit(root, root, &mut tree)?;
    Ok(tree)
}

pub fn apply(source: &Path, game_dir: &Path, progress: impl Fn(usize, usize)) -> Result<()> {
    apply_in_order(&[source], game_dir, progress)
}

pub fn apply_in_order(
    sources: &[&Path],
    game_dir: &Path,
    progress: impl Fn(usize, usize),
) -> Result<()> {
    let game_dir = dunce::canonicalize(game_dir)?;
    let mut directories = Vec::new();
    let mut files = Vec::new();
    for source in sources {
        let source = dunce::canonicalize(source)?;
        ensure!(
            !source.starts_with(&game_dir) && !game_dir.starts_with(&source),
            "Папка патча должна находиться отдельно от папки игры"
        );
        let tree = tree(&source)?;
        directories.extend(tree.directories);
        files.extend(
            tree.files
                .into_iter()
                .map(|relative| (source.join(&relative), relative)),
        );
    }
    ensure!(!files.is_empty(), "В патче нет файлов");
    let mut stage = WorkDir::create(&game_dir, ".nightreign-update")?;
    let staged = stage.path.join("new");
    let backups = stage.path.join("original");
    fs::create_dir(&staged)?;
    fs::create_dir(&backups)?;

    for relative in &directories {
        let target = game_dir.join(relative);
        if let Some(meta) = metadata_if_present(&target)? {
            ensure!(
                meta.is_dir() && !game::is_reparse(&meta),
                "Конфликт папки: {}",
                target.display()
            );
        }
    }
    for (index, (source, relative)) in files.iter().enumerate() {
        let target = game_dir.join(relative);
        if let Some(meta) = metadata_if_present(&target)? {
            ensure!(
                meta.is_file() && !game::is_reparse(&meta),
                "Конфликт файла: {}",
                target.display()
            );
        }
        let mut input = File::open(source)?;
        let mut output = File::create_new(staged.join(index.to_string()))?;
        io::copy(&mut input, &mut output)
            .with_context(|| format!("Не удалось подготовить {}", relative.display()))?;
        output.sync_all()?;
    }

    struct Change {
        destination: PathBuf,
        backup: Option<PathBuf>,
        installed: bool,
    }
    let mut created = Vec::new();
    let mut changes = Vec::new();
    let install = (|| -> Result<()> {
        for relative in &directories {
            let target = game_dir.join(relative);
            if metadata_if_present(&target)?.is_none() {
                fs::create_dir(&target)?;
                created.push(target);
            } else {
                game::ordinary_dir(&target)?;
            }
        }
        for (index, (_, relative)) in files.iter().enumerate() {
            let destination = game_dir.join(relative);
            let backup = if metadata_if_present(&destination)?.is_some() {
                game::ordinary_file(&destination)?;
                // Two packages can replace the same path. Keep each predecessor for rollback.
                let backup = backups.join(index.to_string()).join(relative);
                fs::create_dir_all(backup.parent().unwrap())?;
                fs::rename(&destination, &backup)
                    .with_context(|| format!("Не удалось сохранить {}", destination.display()))?;
                Some(backup)
            } else {
                None
            };
            changes.push(Change {
                destination,
                backup,
                installed: false,
            });
            let change = changes.last_mut().unwrap();
            fs::rename(staged.join(index.to_string()), &change.destination)
                .with_context(|| format!("Не удалось заменить {}", change.destination.display()))?;
            change.installed = true;
            progress(index + 1, files.len());
        }
        Ok(())
    })();

    if let Err(error) = install {
        let mut rollback_errors = Vec::new();
        for change in changes.into_iter().rev() {
            let restore = (|| -> io::Result<()> {
                if change.installed {
                    fs::remove_file(&change.destination)?;
                }
                if let Some(backup) = change.backup {
                    fs::rename(backup, &change.destination)?;
                }
                Ok(())
            })();
            if let Err(error) = restore {
                rollback_errors.push(format!("{}: {error}", change.destination.display()));
            }
        }
        for path in created.into_iter().rev() {
            if let Err(error) = fs::remove_dir(&path) {
                rollback_errors.push(format!("{}: {error}", path.display()));
            }
        }
        if !rollback_errors.is_empty() {
            stage.keep = true;
            return Err(anyhow!(
                "{error:#}. Не удалось полностью восстановить файлы. Резервные копии: {}. {}",
                stage.path.display(),
                rollback_errors.join(" | ")
            ));
        }
        return Err(error).context("Замены отменены");
    }
    Ok(())
}

pub(crate) fn metadata_if_present(path: &Path) -> io::Result<Option<fs::Metadata>> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => Ok(Some(metadata)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}
