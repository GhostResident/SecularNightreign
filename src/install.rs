use anyhow::{Context, Result, anyhow, bail};
use regex::Regex;
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::Read,
    path::Path,
};

use crate::{config::VANILLA_REGULATION_SHA, platform};

pub fn sha256_file(path: &Path) -> Result<String> {
    let mut file =
        File::open(path).with_context(|| format!("не удалось открыть {}", path.display()))?;
    let mut sha = Sha256::new();
    let mut buffer = vec![0_u8; 1024 * 1024];
    loop {
        let count = file.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        sha.update(&buffer[..count]);
    }
    Ok(hex::encode_upper(sha.finalize()))
}

fn copy_if_changed(source: &Path, destination: &Path) -> Result<bool> {
    if destination.is_file() && sha256_file(source)? == sha256_file(destination)? {
        return Ok(false);
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::copy(source, destination).with_context(|| {
        format!(
            "не удалось скопировать {} в {}",
            source.display(),
            destination.display()
        )
    })?;
    Ok(true)
}

pub fn install_package(package: &Path, game: &Path, mut log: impl FnMut(String)) -> Result<String> {
    if platform::game_is_running() {
        bail!("закройте игру перед установкой");
    }
    let files = package.join("mod");
    let version = fs::read_to_string(package.join("version.txt"))?
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned();
    let backup = game.join("secular_backup");
    fs::create_dir_all(&backup)?;

    let regulation = game.join("regulation.bin");
    let package_regulation = files.join("regulation.bin");
    let regulation_backup = backup.join("regulation.bin");
    let marker = backup.join("installed_regulation.txt");
    let current_sha = sha256_file(&regulation)?;
    let mod_sha = sha256_file(&package_regulation)?;
    let backup_sha = if regulation_backup.is_file() {
        Some(sha256_file(&regulation_backup)?)
    } else {
        None
    };
    let previous_mod_sha = fs::read_to_string(&marker)
        .ok()
        .map(|s| s.trim().to_owned());

    if current_sha != mod_sha {
        let original = current_sha.eq_ignore_ascii_case(VANILLA_REGULATION_SHA)
            || backup_sha.as_deref() == Some(current_sha.as_str());
        let previous = previous_mod_sha.as_deref() == Some(current_sha.as_str());
        if !original && !previous {
            bail!(
                "regulation.bin отличается от версии игры 1.03.3, под которую собран мод. Для безопасности установка остановлена до изменения файлов"
            );
        }
        if original && !regulation_backup.is_file() {
            fs::copy(&regulation, &regulation_backup)?;
            log("Резервная копия regulation.bin сохранена".into());
        }
        fs::copy(&package_regulation, &regulation)?;
        log("Установлен regulation.bin".into());
    }
    fs::write(&marker, &mod_sha)?;

    let dll = game.join("dinput8.dll");
    let package_dll = files.join("dinput8.dll");
    if dll.is_file()
        && !backup.join("dinput8.dll").is_file()
        && sha256_file(&dll)? != sha256_file(&package_dll)?
        && !game.join("secular_text.bin").is_file()
    {
        fs::copy(&dll, backup.join("dinput8.dll"))?;
        log("Существующий dinput8.dll сохранён в secular_backup".into());
    }
    for name in ["dinput8.dll", "secular_text.bin", "secular_mute.txt"] {
        if copy_if_changed(&files.join(name), &game.join(name))? {
            log(format!("Установлен {name}"));
        }
    }

    set_russian_language(game, &backup, &mut log)?;
    let home = game.join("SecularMod");
    fs::create_dir_all(&home)?;
    if !platform::same_path(package, &home) {
        let installed_mod = home.join("mod");
        if installed_mod.is_dir() {
            fs::remove_dir_all(&installed_mod)?;
        }
        copy_tree(&files, &installed_mod)?;
    }
    for name in ["version.txt", "ПРОЧТИ.txt"] {
        let source = package.join(name);
        if source.is_file() {
            fs::copy(source, home.join(name))?;
        }
    }
    let launcher = platform::copy_launcher(game)?;
    platform::create_shortcut(game, &launcher)?;
    crate::state::save(&crate::state::SavedState {
        game_dir: game.to_string_lossy().into_owned(),
    })?;
    log("Создан ярлык на рабочем столе".into());
    Ok(version)
}

fn copy_tree(source: &Path, destination: &Path) -> Result<()> {
    fs::create_dir_all(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let from = entry.path();
        let to = destination.join(entry.file_name());
        if from.is_dir() {
            copy_tree(&from, &to)?;
        } else {
            fs::copy(from, to)?;
        }
    }
    Ok(())
}

fn set_russian_language(game: &Path, backup: &Path, log: &mut impl FnMut(String)) -> Result<()> {
    let expression = Regex::new(r"(?m)^#?[ \t]*Language=[^\r\n]*")?;
    for name in ["steam_emu.ini", "OnlineFix.ini"] {
        let path = game.join(name);
        if !path.is_file() {
            continue;
        }
        let bytes = fs::read(&path)?;
        let original = String::from_utf8_lossy(&bytes).into_owned();
        let changed = expression
            .replace_all(&original, "Language=russian")
            .into_owned();
        if changed != original {
            let saved = backup.join(name);
            if !saved.is_file() {
                fs::copy(&path, &saved)?;
            }
            fs::write(&path, changed.as_bytes())?;
            log(format!("Язык игры: русский ({name})"));
        }
    }
    Ok(())
}

pub fn installed_version(game: &Path) -> Option<String> {
    fs::read_to_string(game.join("SecularMod/version.txt"))
        .ok()
        .map(|s| s.trim_start_matches('\u{feff}').trim().to_owned())
}

pub fn installed_files_present(game: &Path) -> bool {
    [
        "regulation.bin",
        "dinput8.dll",
        "secular_text.bin",
        "secular_mute.txt",
    ]
    .iter()
    .all(|name| game.join(name).is_file())
}

pub fn uninstall(game: &Path, mut log: impl FnMut(String)) -> Result<bool> {
    if platform::game_is_running() {
        bail!("закройте игру перед удалением мода");
    }
    let backup = game.join("secular_backup");
    for name in [
        "dinput8.dll",
        "secular_text.bin",
        "secular_mute.txt",
        "secular_config.ini",
        "secular_text.log",
    ] {
        if fs::remove_file(game.join(name)).is_ok() {
            log(format!("Удалён {name}"));
        }
    }
    for name in [
        "regulation.bin",
        "dinput8.dll",
        "steam_emu.ini",
        "OnlineFix.ini",
    ] {
        let saved = backup.join(name);
        if saved.is_file() {
            fs::copy(&saved, game.join(name))?;
            log(format!("Восстановлен {name}"));
        }
    }
    if !backup.join("regulation.bin").is_file() {
        log("Резервной копии regulation.bin нет; при необходимости проверьте файлы игры".into());
    }
    if backup.is_dir() {
        fs::remove_dir_all(&backup)?;
    }
    platform::remove_shortcut();
    crate::state::clear();
    let home = game.join("SecularMod");
    let current = std::env::current_exe()?;
    let running_from_home = current.starts_with(&home);
    if home.is_dir() {
        if running_from_home {
            platform::schedule_self_directory_removal(&home)?;
        } else {
            fs::remove_dir_all(&home)?;
        }
    }
    Ok(running_from_home)
}

pub fn require_game(game: &Path) -> Result<()> {
    if !game.join(crate::config::GAME_EXE).is_file() {
        return Err(anyhow!("в выбранной папке нет {}", crate::config::GAME_EXE));
    }
    Ok(())
}
