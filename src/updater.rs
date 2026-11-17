use std::{
    collections::HashMap,
    env,
    fs::{self, File},
    io::{self, BufWriter, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::{Instant, SystemTime},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use reqwest::Url;
use serde::{Deserialize, Serialize};

use crate::{download, game, payload, platform};

const DATA: &str = "elamigos-1.bin";

pub struct Patch {
    pub from: &'static str,
    pub to: &'static str,
    installer: &'static str,
    size: &'static str,
    direct: bool,
}

static PATCHES: [Patch; 3] = [
    Patch {
        from: "1.3.0",
        to: "1.3.1",
        installer: "Elden Ring Nightreign update 1.03.0 - 1.03.1.exe",
        size: "≈ 2,5 ГБ",
        direct: false,
    },
    Patch {
        from: "1.3.1",
        to: "1.3.2",
        installer: "Elden Ring Nightreign update 1.03.1 - 1.03.2.exe",
        size: "≈ 1,8 ГБ",
        direct: false,
    },
    Patch {
        from: "1.3.2",
        to: "1.3.3",
        installer: "",
        size: "≈ 150 МБ",
        direct: true,
    },
];

#[derive(Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MirrorId {
    Multipart,
    Archive,
}

enum Source {
    Multipart { installer: Url, parts: [Url; 2] },
    Archive(Url),
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Step {
    from: &'static str,
    to: &'static str,
    size: &'static str,
    direct: bool,
    mirrors: Vec<MirrorId>,
}

#[derive(Serialize)]
pub struct Package {
    pub path: PathBuf,
    pub name: String,
    kind: PackageKind,
}

#[derive(Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
enum PackageKind {
    Installer,
    Archive,
    Files,
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub from: &'static str,
    phase: &'static str,
    message: Option<String>,
    fraction: Option<f64>,
    bytes: Option<u64>,
    total: Option<u64>,
    bytes_per_second: Option<u64>,
    elapsed: Option<u64>,
}

impl Progress {
    fn phase(patch: &Patch, phase: &'static str) -> Self {
        Self {
            from: patch.from,
            phase,
            message: None,
            fraction: None,
            bytes: None,
            total: None,
            bytes_per_second: None,
            elapsed: None,
        }
    }

    fn log(patch: &Patch, phase: &'static str, message: impl Into<String>) -> Self {
        Self {
            message: Some(message.into()),
            ..Self::phase(patch, phase)
        }
    }
}

pub fn patch(from: &str) -> Result<&'static Patch> {
    PATCHES
        .iter()
        .find(|p| p.from == from)
        .context("Неизвестный переход версии")
}

pub fn plan(version: &str) -> Result<Vec<Step>> {
    let index = game::CHAIN
        .iter()
        .position(|&v| v == version)
        .context("Версия вне цепочки обновления")?;
    if index == PATCHES.len() {
        return Ok(Vec::new());
    }
    let config = Config::load()?;
    Ok(PATCHES
        .iter()
        .skip(index)
        .filter(|p| !p.direct || p.from == version)
        .map(|p| Step {
            from: p.from,
            to: p.to,
            size: p.size,
            direct: p.direct,
            mirrors: [MirrorId::Multipart, MirrorId::Archive]
                .into_iter()
                .filter(|&id| config.source(p, id).is_some())
                .collect(),
        })
        .collect())
}

pub fn config_path() -> Result<PathBuf> {
    let exe = env::current_exe()?;
    Ok(exe.with_file_name(".env"))
}

pub fn downloads_dir() -> Result<PathBuf> {
    if let Some(configured) = Config::load()
        .ok()
        .and_then(|config| config.value("NIGHTREIGN_DOWNLOADS_DIR"))
    {
        let program_dir = config_path()?;
        return Ok(program_dir
            .parent()
            .context("Папка программы недоступна")?
            .join(configured));
    }
    let root = platform::downloads().or_else(|error| {
        env::var_os("USERPROFILE")
            .map(|profile| PathBuf::from(profile).join("Downloads"))
            .ok_or(error)
    })?;
    Ok(root.join("NightreignUpdates"))
}

struct Config(HashMap<String, String>);

impl Config {
    fn load() -> Result<Self> {
        let path = config_path()?;
        let contents = match fs::read_to_string(&path) {
            Ok(contents) => contents,
            Err(error) if error.kind() == io::ErrorKind::NotFound => String::new(),
            Err(error) => {
                return Err(error).context(format!("Не удалось прочитать {}", path.display()));
            }
        };
        let mut values = HashMap::new();
        for line in contents.trim_start_matches('\u{feff}').lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let line = line.strip_prefix("export ").unwrap_or(line);
            if let Some((key, text)) = line.split_once('=') {
                let text = text.trim();
                let value = text
                    .strip_prefix('"')
                    .and_then(|v| v.split_once('"').map(|(v, _)| v))
                    .or_else(|| {
                        text.strip_prefix('\'')
                            .and_then(|v| v.split_once('\'').map(|(v, _)| v))
                    })
                    .unwrap_or_else(|| text.split_once(" #").map_or(text, |(v, _)| v))
                    .trim();
                values.insert(key.trim().to_owned(), value.to_owned());
            }
        }
        Ok(Self(values))
    }

    fn value(&self, key: &str) -> Option<String> {
        env::var(key)
            .ok()
            .or_else(|| self.0.get(key).cloned())
            .map(|value| value.trim().to_owned())
            .filter(|value| !value.is_empty())
    }

    fn url(&self, key: &str) -> Option<Url> {
        let url = Url::parse(&self.value(key)?).ok()?;
        (matches!(url.scheme(), "http" | "https") && url.has_host()).then_some(url)
    }

    fn source(&self, patch: &Patch, id: MirrorId) -> Option<Source> {
        let base = format!(
            "NIGHTREIGN_{}_TO_{}",
            patch.from.replace('.', "_"),
            patch.to.replace('.', "_")
        );
        let url = |suffix| self.url(&format!("{base}_{suffix}"));
        match id {
            MirrorId::Multipart if !patch.direct => Some(Source::Multipart {
                installer: url("EXE_URL")?,
                parts: [url("BIN_PART1_URL")?, url("BIN_PART2_URL")?],
            }),
            MirrorId::Archive => Some(Source::Archive(url("URL")?)),
            _ => None,
        }
    }
}

pub fn locate(patch: &Patch, path: &Path) -> Result<Package> {
    ensure!(!path.as_os_str().is_empty(), "Укажите папку или файл патча");
    let path =
        dunce::canonicalize(path).with_context(|| format!("Не найдено: {}", path.display()))?;
    let (path, kind) = if path.is_dir() && patch.direct {
        (payload::find_root(&path)?, PackageKind::Files)
    } else if path.is_dir() {
        (find_installer(patch, &path)?, PackageKind::Installer)
    } else if has_extension(&path, "zip") || has_extension(&path, "rar") {
        game::ordinary_file(&path)?;
        (path, PackageKind::Archive)
    } else if !patch.direct && has_extension(&path, "exe") {
        ensure!(!is_game_exe(&path), "Выберите установщик патча");
        game::check_pe(&path)?;
        (path, PackageKind::Installer)
    } else {
        bail!(
            "Выберите {} или архив ZIP/RAR",
            if patch.direct {
                "папку с файлами патча"
            } else {
                "установщик EXE"
            }
        );
    };
    let name = path
        .file_name()
        .context("Не задано имя пакета")?
        .to_string_lossy()
        .into_owned();
    Ok(Package { path, name, kind })
}

fn has_extension(path: &Path, extension: &str) -> bool {
    path.extension()
        .is_some_and(|value| value.eq_ignore_ascii_case(extension))
}

fn is_game_exe(path: &Path) -> bool {
    path.file_name().is_some_and(|name| {
        name.eq_ignore_ascii_case("nightreign.exe")
            || name.eq_ignore_ascii_case("start_protected_game.exe")
    })
}

fn find_installer(patch: &Patch, root: &Path) -> Result<PathBuf> {
    fn visit(dir: &Path, depth: u8, executables: &mut Vec<PathBuf>) -> Result<()> {
        for entry in fs::read_dir(dir)? {
            let entry = entry?;
            let path = entry.path();
            let metadata = fs::symlink_metadata(&path)?;
            if game::is_reparse(&metadata) {
                continue;
            }
            if metadata.is_dir() && depth < 3 {
                visit(&path, depth + 1, executables)?;
            } else if metadata.is_file() && has_extension(&path, "exe") && !is_game_exe(&path) {
                executables.push(path);
            }
        }
        Ok(())
    }
    game::ordinary_dir(root)?;
    let mut candidates = Vec::new();
    visit(root, 0, &mut candidates)?;
    let exact = candidates.iter().filter(|p| {
        p.file_name()
            .is_some_and(|name| name.eq_ignore_ascii_case(patch.installer))
    });
    let nightreign = candidates.iter().filter(|p| {
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_ascii_lowercase();
        name.contains("nightreign")
    });
    let picks: Vec<_> = exact.collect();
    let picks = if picks.is_empty() {
        nightreign.collect()
    } else {
        picks
    };
    let installer = match (picks.as_slice(), candidates.as_slice()) {
        ([one], _) => (*one).clone(),
        ([], [one]) => one.clone(),
        ([], []) => bail!("В папке не найден установщик EXE"),
        _ => bail!("Найдено несколько установщиков. Укажите точный путь к EXE."),
    };
    game::check_pe(&installer)?;
    Ok(installer)
}

pub fn prepare(
    patch: &Patch,
    mirror: MirrorId,
    cache: &Path,
    report: &impl Fn(Progress),
) -> Result<Package> {
    let source = Config::load()?
        .source(patch, mirror)
        .context("Зеркало не настроено. Проверьте .env рядом с программой.")?;
    let mut folder = WorkDir::create(cache, &format!("{}-{}", patch.from, patch.to))?;
    let client = download::client()?;

    let fetch = |url: &Url, path: &Path, part: usize, count: usize| {
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        report(Progress::log(patch, "download", format!("Загрузка {name}")));
        let started = Instant::now();
        download::fetch(
            &client,
            url,
            path,
            |message| report(Progress::log(patch, "download", message)),
            |bytes, total| {
                let speed = (bytes as f64 / started.elapsed().as_secs_f64().max(0.1)) as u64;
                report(Progress {
                    fraction: total.and_then(|len| {
                        (len > 0)
                            .then_some((part as f64 + bytes as f64 / len as f64) / count as f64)
                    }),
                    bytes: Some(bytes),
                    total,
                    bytes_per_second: Some(speed),
                    ..Progress::phase(patch, "download")
                });
            },
        )
    };

    let package = match source {
        Source::Multipart { installer, parts } => {
            let executable = folder.path.join(patch.installer);
            fetch(&installer, &executable, 0, 3)?;
            game::check_pe(&executable)?;
            let part1 = folder.path.join(format!("{DATA}.part1"));
            let part2 = folder.path.join(format!("{DATA}.part2"));
            fetch(&parts[0], &part1, 1, 3)?;
            fetch(&parts[1], &part2, 2, 3)?;
            report(Progress::log(patch, "assemble", format!("Сборка {DATA}")));
            assemble(&[part1, part2], &folder.path.join(DATA))?;
            locate(patch, &executable)?
        }
        Source::Archive(url) => {
            let file = folder.path.join("patch.download");
            fetch(&url, &file, 0, 1)?;
            let ext = payload::archive_extension(&file)?;
            let archive = folder
                .path
                .join(format!("nightreign-patch-{}.{ext}", patch.to));
            fs::rename(file, &archive)?;
            extract_package(patch, &archive, &folder.path.join("extracted"), report)?
        }
    };
    folder.keep = true;
    report(Progress::log(
        patch,
        "ready",
        format!("Пакет готов: {}", package.name),
    ));
    Ok(package)
}

fn assemble(parts: &[PathBuf], destination: &Path) -> Result<()> {
    let partial = destination.with_extension("bin.partial");
    let mut whole = BufWriter::with_capacity(256 * 1024, File::create_new(&partial)?);
    for part in parts {
        let mut input = File::open(part)?;
        io::copy(&mut input, &mut whole)?;
    }
    whole.flush()?;
    whole.get_ref().sync_all()?;
    drop(whole);
    fs::rename(partial, destination)?;
    for part in parts {
        fs::remove_file(part)?;
    }
    Ok(())
}

fn extract_package(
    patch: &Patch,
    archive: &Path,
    destination: &Path,
    report: &impl Fn(Progress),
) -> Result<Package> {
    report(Progress::log(
        patch,
        "extract",
        format!("Распаковка {}", archive.display()),
    ));
    payload::extract(archive, destination)?;
    locate(patch, destination)
}

pub fn install(
    patch: &Patch,
    input: &Path,
    game: &game::Game,
    cache: &Path,
    report: &impl Fn(Progress),
) -> Result<bool> {
    let mut package = locate(patch, input)?;
    let extraction;
    if package.kind == PackageKind::Archive {
        extraction = WorkDir::create(cache, "local")?;
        package = extract_package(
            patch,
            &package.path,
            &extraction.path.join("extracted"),
            report,
        )?;
    }
    let file_version = |exe: &Path| platform::file_version(exe).and_then(|v| game::normalize(&v));
    if patch.direct {
        let executable = package.path.join("nightreign.exe");
        game::check_pe(&executable)?;
        if let Some(version) = file_version(&executable) {
            ensure!(
                version == patch.to,
                "Пакет содержит версию {version}. Нужна {}.",
                patch.to
            );
        }
        report(Progress::log(
            patch,
            "apply",
            format!("Копирование файлов в {}", game.game_dir.display()),
        ));
        payload::apply(&package.path, &game.game_dir, |copied, total| {
            report(Progress {
                fraction: Some(copied as f64 / total as f64),
                ..Progress::phase(patch, "apply")
            });
        })?;
    } else {
        let initial = folder_bytes(&game.root, 6);
        report(Progress::log(
            patch,
            "install",
            format!("Запуск {}", package.path.display()),
        ));
        platform::install(&package.path, &game.root, |elapsed, message| {
            report(Progress {
                elapsed: Some(elapsed),
                bytes: Some(folder_bytes(&game.root, 6).saturating_sub(initial)),
                message: message.map(str::to_owned),
                ..Progress::phase(patch, "install")
            });
        })?;
    }
    let result = file_version(&game.game_dir.join("nightreign.exe"));
    if let Some(version) = &result {
        ensure!(
            version == patch.to,
            "После установки найдена версия {version}. Ожидалась {}.",
            patch.to
        );
    }
    let message = if result.is_some() {
        format!("Версия {} подтверждена", patch.to)
    } else {
        "Установка завершена. Не удалось прочитать версию nightreign.exe.".to_owned()
    };
    report(Progress::log(patch, "done", message));
    Ok(result.is_some())
}

fn folder_bytes(folder: &Path, depth: u8) -> u64 {
    let Ok(entries) = fs::read_dir(folder) else {
        return 0;
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let metadata = fs::symlink_metadata(entry.path()).ok()?;
            if game::is_reparse(&metadata) {
                return None;
            }
            if metadata.is_file() {
                Some(metadata.len())
            } else if metadata.is_dir() && depth > 0 {
                Some(folder_bytes(&entry.path(), depth - 1))
            } else {
                None
            }
        })
        .sum()
}

pub struct WorkDir {
    pub path: PathBuf,
    pub keep: bool,
}

impl WorkDir {
    pub fn create(parent: &Path, name: &str) -> Result<Self> {
        static SEQUENCE: AtomicU64 = AtomicU64::new(0);
        let time = SystemTime::UNIX_EPOCH.elapsed()?.as_millis();
        let id = SEQUENCE.fetch_add(1, Ordering::Relaxed);
        let path = parent.join(format!("{name}-{time}-{}-{id}", std::process::id()));
        fs::create_dir(&path).map_err(|e| anyhow!("Не удалось создать {}: {e}", path.display()))?;
        Ok(Self { path, keep: false })
    }
}

impl Drop for WorkDir {
    fn drop(&mut self) {
        if !self.keep {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}
