use anyhow::{Context, Result, bail};
use std::{
    fs,
    path::Path,
    process::{Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use crate::{
    config::{ElAmigosPatch, GAME_CHAIN, PATCH_131, PATCH_132, PATCH_133_SHA256, PATCH_133_URL},
    download,
    events::Event,
    package, platform, state,
};

pub fn update_game(game: &Path, cancel: &Arc<AtomicBool>, send: &impl Fn(Event)) -> Result<String> {
    let mut version = platform::game_version(game)?;
    send(Event::Log(format!("Версия игры: {version}")));
    let mut index = chain_index(&version).with_context(|| {
        format!("версия игры {version} не поддерживается; ожидается 1.3.0–1.3.3")
    })?;
    let target = GAME_CHAIN.len() - 1;
    while index < target {
        if cancel.load(Ordering::Relaxed) {
            bail!("операция отменена; загруженные части сохранены");
        }
        let from = GAME_CHAIN[index];
        let to = GAME_CHAIN[index + 1];
        send(Event::Stage(format!("Обновление игры {from} → {to}")));
        send(Event::Log(format!("Начат патч {from} → {to}")));
        match to {
            "1.3.1" => apply_elamigos(&PATCH_131, game, cancel, send)?,
            "1.3.2" => apply_elamigos(&PATCH_132, game, cancel, send)?,
            "1.3.3" => apply_133(game, cancel, send)?,
            _ => bail!("неизвестный переход версии {from} → {to}"),
        }
        version = platform::game_version(game)?;
        let actual = chain_index(&version);
        if actual != Some(index + 1) {
            bail!("после патча версия игры {version}, ожидалась {to}; патч не применился");
        }
        send(Event::Log(format!("Версия подтверждена: {version}")));
        index += 1;
    }
    Ok(version)
}

fn chain_index(version: &str) -> Option<usize> {
    GAME_CHAIN
        .iter()
        .position(|candidate| *candidate == version)
        .or_else(|| {
            GAME_CHAIN.iter().position(|candidate| {
                candidate.starts_with(&format!("{version}."))
                    || version.starts_with(&format!("{candidate}."))
            })
        })
}

fn apply_elamigos(
    patch: &ElAmigosPatch,
    game: &Path,
    cancel: &Arc<AtomicBool>,
    send: &impl Fn(Event),
) -> Result<()> {
    let dir = state::downloads_dir().join(format!("patch-{}", patch.to));
    fs::create_dir_all(&dir)?;
    let installer = dir.join(patch.installer_name);
    send(Event::Stage(format!(
        "Скачивание установщика {}…",
        patch.to
    )));
    download::download_resumable(patch.installer_url, &installer, None, cancel, send)?;

    let mut parts = Vec::new();
    for (index, url) in patch.part_urls.iter().enumerate() {
        let part = dir.join(format!("elamigos-1.bin.part{}", index + 1));
        send(Event::Stage(format!(
            "Скачивание данных {}: часть {}/2…",
            patch.to,
            index + 1
        )));
        download::download_resumable(url, &part, None, cancel, send)?;
        parts.push(part);
    }
    let bin = dir.join("elamigos-1.bin");
    send(Event::Stage("Склейка elamigos-1.bin…".into()));
    download::concatenate(&parts, &bin, cancel)?;
    send(Event::Log("Части elamigos-1.bin успешно объединены".into()));

    run_installer(&installer, platform::install_root(game), cancel, send)?;
    fs::remove_file(&installer).ok();
    fs::remove_file(&bin).ok();
    for part in parts {
        fs::remove_file(part).ok();
    }
    Ok(())
}

fn apply_133(game: &Path, cancel: &Arc<AtomicBool>, send: &impl Fn(Event)) -> Result<()> {
    let zip = state::downloads_dir().join("patch-1.3.3/nightreign-patch-1.03.3.zip");
    send(Event::Stage("Скачивание патча 1.03.3…".into()));
    download::download_resumable(PATCH_133_URL, &zip, Some(PATCH_133_SHA256), cancel, send)?;
    if cancel.load(Ordering::Relaxed) {
        bail!("операция отменена; патч сохранён");
    }
    send(Event::Stage("Распаковка патча 1.03.3…".into()));
    let temp = tempfile::Builder::new()
        .prefix("Nightreign_133_")
        .tempdir()?;
    package::extract_zip_to(&zip, temp.path())?;
    if !temp.path().join("nightreign.exe").is_file() {
        bail!("в патче 1.03.3 нет nightreign.exe; игра не изменена");
    }
    overlay_tree(temp.path(), temp.path(), game)?;
    Ok(())
}

fn overlay_tree(root: &Path, current: &Path, destination_root: &Path) -> Result<()> {
    for entry in fs::read_dir(current)? {
        let entry = entry?;
        let source = entry.path();
        if source.is_dir() {
            overlay_tree(root, &source, destination_root)?;
            continue;
        }
        let relative = source.strip_prefix(root)?;
        let destination = destination_root.join(relative);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::copy(&source, &destination)
            .with_context(|| format!("не удалось заменить {}", destination.display()))?;
    }
    Ok(())
}

fn run_installer(
    installer: &Path,
    install_root: &Path,
    cancel: &Arc<AtomicBool>,
    send: &impl Fn(Event),
) -> Result<()> {
    send(Event::Stage("Установка патча и проверка файлов…".into()));
    send(Event::Log(format!("Запуск {}", installer.display())));
    let mut child = Command::new(installer)
        .args([
            "/VERYSILENT",
            "/SUPPRESSMSGBOXES",
            "/NOCANCEL",
            "/NORESTART",
            &format!("/DIR={}", install_root.display()),
        ])
        .current_dir(installer.parent().unwrap_or(install_root))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .context("не удалось запустить установщик патча")?;
    let started = Instant::now();
    let mut last_rapid_check = Instant::now() - Duration::from_secs(10);
    loop {
        if cancel.load(Ordering::Relaxed) {
            child.kill().ok();
            bail!("установка патча отменена; скачанные файлы сохранены");
        }
        if let Some(status) = child.try_wait()? {
            let code = status.code().unwrap_or(0);
            if code != 0 && code != 3010 {
                bail!("установщик патча завершился с кодом {code}");
            }
            send(Event::Log(format!(
                "Установщик патча завершён с кодом {code}"
            )));
            return Ok(());
        }
        if started.elapsed() > Duration::from_secs(40 * 60) {
            child.kill().ok();
            bail!("установщик патча не завершился за 40 минут");
        }
        if last_rapid_check.elapsed() >= Duration::from_secs(4) {
            if close_rapidcrc_if_done() {
                send(Event::Log(
                    "RapidCRC завершил проверку; окно закрыто автоматически".into(),
                ));
            }
            last_rapid_check = Instant::now();
        }
        thread::sleep(Duration::from_secs(1));
    }
}

fn close_rapidcrc_if_done() -> bool {
    let script = r#"
Add-Type @'
using System;
using System.Text;
using System.Runtime.InteropServices;
public class SecularRapid {
  public delegate bool P(IntPtr h, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumWindows(P p, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, P p, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder s, int n);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);
}
'@
$w=[IntPtr]::Zero
[SecularRapid]::EnumWindows({param($h,$l);$s=New-Object Text.StringBuilder 256;[void][SecularRapid]::GetWindowText($h,$s,256);if([SecularRapid]::IsWindowVisible($h)-and $s.ToString().IndexOf('RapidCRC',[StringComparison]::OrdinalIgnoreCase)-ge 0){$script:w=$h;return $false};return $true},[IntPtr]::Zero)|Out-Null
if($w-eq[IntPtr]::Zero){exit 2}
$a=New-Object Text.StringBuilder
[SecularRapid]::EnumChildWindows($w,{param($h,$l);$s=New-Object Text.StringBuilder 512;[void][SecularRapid]::GetWindowText($h,$s,512);if($s.Length){[void]$script:a.Append($s).Append(' | ')};return $true},[IntPtr]::Zero)|Out-Null
if($a.ToString()-match '(\d+)\s*/\s*(\d+)\s*done'-and $matches[1]-eq$matches[2]){[void][SecularRapid]::PostMessage($w,0x10,[IntPtr]::Zero,[IntPtr]::Zero);exit 0};exit 3
"#;
    Command::new("powershell.exe")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            script,
        ])
        .creation_flags_hidden(0x0800_0000)
        .status()
        .is_ok_and(|status| status.success())
}

#[cfg(windows)]
trait HiddenCommand {
    fn creation_flags_hidden(&mut self, flags: u32) -> &mut Self;
}
#[cfg(windows)]
impl HiddenCommand for Command {
    fn creation_flags_hidden(&mut self, flags: u32) -> &mut Self {
        std::os::windows::process::CommandExt::creation_flags(self, flags);
        self
    }
}

#[cfg(not(windows))]
trait HiddenCommand {
    fn creation_flags_hidden(&mut self, _flags: u32) -> &mut Self;
}
#[cfg(not(windows))]
impl HiddenCommand for Command {
    fn creation_flags_hidden(&mut self, _flags: u32) -> &mut Self {
        self
    }
}

#[cfg(test)]
mod tests {
    use super::chain_index;

    #[test]
    fn recognizes_supported_versions() {
        assert_eq!(chain_index("1.3.0"), Some(0));
        assert_eq!(chain_index("1.3.3"), Some(3));
        assert_eq!(chain_index("1.4.0"), None);
    }
}
