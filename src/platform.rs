use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
use sysinfo::{ProcessesToUpdate, System};

use crate::config::{APP_EXE, GAME_EXE, GAME_PROCESS, SEAMLESS_EXE, SHORTCUT_NAME};

pub fn normalize_game_dir(input: &Path) -> Option<PathBuf> {
    let direct = input.join(GAME_EXE);
    if direct.is_file() {
        return fs::canonicalize(input)
            .ok()
            .or_else(|| Some(input.to_path_buf()));
    }
    let nested = input.join("Game");
    if nested.join(GAME_EXE).is_file() {
        return fs::canonicalize(&nested).ok().or(Some(nested));
    }
    None
}

pub fn game_is_running() -> bool {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    system.processes().values().any(|p| {
        p.name()
            .to_string_lossy()
            .eq_ignore_ascii_case(GAME_PROCESS)
    })
}

pub fn launch_game(game: &Path) -> Result<()> {
    let seamless = game.join(SEAMLESS_EXE);
    let executable = if seamless.is_file() {
        seamless
    } else {
        game.join(GAME_EXE)
    };
    Command::new(&executable)
        .current_dir(game)
        .spawn()
        .with_context(|| format!("не удалось запустить {}", executable.display()))?;
    Ok(())
}

pub fn game_version(game: &Path) -> Result<String> {
    let exe = game.join(GAME_EXE);
    let escaped = exe.to_string_lossy().replace('\'', "''");
    let script = format!("(Get-Item -LiteralPath '{escaped}').VersionInfo.FileVersion");
    let output = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(0x0800_0000)
        .output()
        .context("не удалось прочитать версию nightreign.exe")?;
    if !output.status.success() {
        bail!("Windows не смог прочитать версию nightreign.exe");
    }
    let raw = String::from_utf8_lossy(&output.stdout);
    let mut numbers: Vec<u32> = raw
        .trim()
        .split('.')
        .filter_map(|part| part.trim().parse().ok())
        .collect();
    if numbers.is_empty() {
        bail!("неизвестный формат версии: {raw}");
    }
    while numbers.len() > 3 && numbers.last() == Some(&0) {
        numbers.pop();
    }
    while numbers.len() < 3 {
        numbers.push(0);
    }
    Ok(format!("{}.{}.{}", numbers[0], numbers[1], numbers[2]))
}

pub fn install_root(game: &Path) -> &Path {
    if game
        .file_name()
        .is_some_and(|n| n.to_string_lossy().eq_ignore_ascii_case("Game"))
    {
        game.parent().unwrap_or(game)
    } else {
        game
    }
}

pub fn open_folder(path: &Path) {
    let _ = Command::new("explorer.exe").arg(path).spawn();
}

fn ps_literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

pub fn create_shortcut(game: &Path, launcher: &Path) -> Result<()> {
    let desktop = dirs::desktop_dir().context("не найдена папка рабочего стола")?;
    let shortcut = desktop.join(SHORTCUT_NAME);
    let script = format!(
        "$w=New-Object -ComObject WScript.Shell;$s=$w.CreateShortcut({});$s.TargetPath={};$s.WorkingDirectory={};$s.IconLocation={};$s.Description='Секулярный мод: обновление и запуск игры';$s.Save()",
        ps_literal(&shortcut.to_string_lossy()),
        ps_literal(&launcher.to_string_lossy()),
        ps_literal(&game.to_string_lossy()),
        ps_literal(&format!("{},0", game.join(GAME_EXE).display())),
    );
    let status = Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            &script,
        ])
        .creation_flags(0x0800_0000)
        .status()
        .context("не удалось создать ярлык")?;
    if !status.success() {
        bail!("PowerShell не смог создать ярлык на рабочем столе");
    }
    Ok(())
}

pub fn remove_shortcut() {
    if let Some(desktop) = dirs::desktop_dir() {
        fs::remove_file(desktop.join(SHORTCUT_NAME)).ok();
    }
}

pub fn copy_launcher(game: &Path) -> Result<PathBuf> {
    let home = game.join("SecularMod");
    fs::create_dir_all(&home)?;
    let destination = home.join(APP_EXE);
    let current = std::env::current_exe()?;
    if !same_path(&current, &destination) {
        fs::copy(&current, &destination).with_context(|| {
            format!("не удалось скопировать лаунчер в {}", destination.display())
        })?;
    }
    Ok(destination)
}

pub fn same_path(a: &Path, b: &Path) -> bool {
    let clean = |p: &Path| {
        p.to_string_lossy()
            .trim_end_matches(['\\', '/'])
            .to_lowercase()
    };
    clean(a) == clean(b)
}

pub fn schedule_self_directory_removal(dir: &Path) -> Result<()> {
    let pid = std::process::id();
    let escaped = dir.to_string_lossy().replace('\'', "''");
    let script = format!(
        "Wait-Process -Id {pid} -ErrorAction SilentlyContinue; Start-Sleep -Milliseconds 300; Remove-Item -LiteralPath '{escaped}' -Recurse -Force -ErrorAction SilentlyContinue"
    );
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &script,
        ])
        .creation_flags(0x0800_0000)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    Ok(())
}

#[cfg(windows)]
trait CommandExtHidden {
    fn creation_flags(&mut self, flags: u32) -> &mut Self;
}

#[cfg(windows)]
impl CommandExtHidden for Command {
    fn creation_flags(&mut self, flags: u32) -> &mut Self {
        std::os::windows::process::CommandExt::creation_flags(self, flags);
        self
    }
}

#[cfg(not(windows))]
trait CommandExtHidden {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self;
}

#[cfg(not(windows))]
impl CommandExtHidden for Command {
    fn creation_flags(&mut self, _flags: u32) -> &mut Self {
        self
    }
}
