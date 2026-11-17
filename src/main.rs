#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod download;
mod game;
mod mod_release;
mod payload;
mod platform;
mod secular;
#[cfg(test)]
mod tests;
mod updater;

use std::{fs, path::Path, sync::Mutex};

use anyhow::{Context, Result, anyhow, bail};
use serde::Serialize;
use tauri::{Emitter, Manager, WebviewWindow, WindowEvent};
use updater::{MirrorId, Progress};

#[derive(Default)]
struct AppOperation(Mutex<()>);

#[derive(Default)]
struct LatestMod(Mutex<Option<mod_release::Release>>);

fn describe(error: anyhow::Error) -> String {
    format!("{error:#}")
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(work)
        .await
        .map_err(|error| error.to_string())?
        .map_err(describe)
}

fn with_operation<T>(window: &WebviewWindow, work: impl FnOnce(&Path) -> Result<T>) -> Result<T> {
    let state = window.state::<AppOperation>();
    let _guard = state
        .0
        .try_lock()
        .map_err(|_| anyhow!("Обновлятор занят"))?;
    let cache = updater::downloads_dir()?;
    fs::create_dir_all(&cache).context("Не удалось создать папку загрузок")?;
    game::ordinary_dir(&cache)?;
    let lock = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(cache.join(".updater.lock"))
        .context("Не удалось открыть блокировку обновлятора")?;
    lock.try_lock()
        .context("Другая копия обновлятора уже работает")?;
    work(&cache)
}

#[tauri::command]
async fn browse(window: WebviewWindow) -> Result<Option<String>, String> {
    let owner = window.hwnd().map_err(|error| error.to_string())?.0 as usize;
    blocking(move || platform::browse_folder(owner)).await
}

#[tauri::command]
async fn verify(path: String) -> Result<game::Game, String> {
    blocking(move || game::inspect(Path::new(path.trim()))).await
}

#[tauri::command]
async fn plan(version: String) -> Result<Vec<updater::Step>, String> {
    blocking(move || updater::plan(&version)).await
}

#[tauri::command]
async fn locate(from: String, path: String) -> Result<updater::Package, String> {
    blocking(move || updater::locate(updater::patch(&from)?, Path::new(path.trim()))).await
}

#[tauri::command]
async fn download_patch(
    window: WebviewWindow,
    from: String,
    mirror: MirrorId,
) -> Result<updater::Package, String> {
    blocking(move || {
        with_operation(&window, |cache| {
            updater::prepare(updater::patch(&from)?, mirror, cache, &|progress| {
                let _ = window.emit("patch-progress", progress);
            })
        })
    })
    .await
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct InstallResult {
    version: String,
    confirmed: bool,
}

#[tauri::command]
async fn install_patch(
    window: WebviewWindow,
    from: String,
    path: String,
    root: String,
) -> Result<InstallResult, String> {
    blocking(move || {
        with_operation(&window, |cache| {
            let patch = updater::patch(&from)?;
            let game = game::inspect(Path::new(&root))?;
            if let Some(version) = &game.version
                && version != &from
            {
                bail!("Патч рассчитан на версию {from}. Найдена версия {version}.");
            }
            let notify = |progress: Progress| {
                let _ = window.emit("patch-progress", progress);
            };
            let confirmed = updater::install(patch, Path::new(&path), &game, cache, &notify)?;
            Ok(InstallResult {
                version: patch.to.to_owned(),
                confirmed,
            })
        })
    })
    .await
}

#[tauri::command]
async fn open_folder(kind: String, root: Option<String>) -> Result<(), String> {
    blocking(move || {
        let path = match kind.as_str() {
            "downloads" => {
                let path = updater::downloads_dir()?;
                fs::create_dir_all(&path)?;
                path
            }
            "config" => updater::config_path()?
                .parent()
                .context("Папка программы недоступна")?
                .to_owned(),
            "game" => {
                game::inspect(Path::new(root.as_deref().context("Выберите папку игры")?))?.root
            }
            _ => bail!("Неизвестная папка"),
        };
        platform::open_folder(&path)
    })
    .await
}

fn selected_mod(window: &WebviewWindow, id: Option<&str>) -> Result<Option<mod_release::Release>> {
    let state = window.state::<LatestMod>();
    let release = state
        .0
        .lock()
        .map_err(|_| anyhow!("Не удалось прочитать выбранный релиз"))?;
    if let Some(id) = id
        && !release
            .as_ref()
            .is_some_and(|offer| offer.id.to_string() == id)
    {
        bail!("Список релизов изменился. Проверьте обновления ещё раз.");
    }
    Ok(release.clone())
}

#[tauri::command]
async fn mod_status(window: WebviewWindow, root: Option<String>) -> Result<secular::View, String> {
    blocking(move || {
        let release = selected_mod(&window, None)?;
        secular::view(
            root.as_deref().map(Path::new),
            release.as_ref(),
            &updater::downloads_dir()?,
        )
    })
    .await
}

#[tauri::command]
async fn mod_check(window: WebviewWindow, root: Option<String>) -> Result<secular::View, String> {
    blocking(move || {
        let release = mod_release::latest()?;
        let view = secular::view(
            root.as_deref().map(Path::new),
            release.as_ref(),
            &updater::downloads_dir()?,
        )?;
        let state = window.state::<LatestMod>();
        *state
            .0
            .lock()
            .map_err(|_| anyhow!("Не удалось сохранить выбранный релиз"))? = release;
        Ok(view)
    })
    .await
}

#[derive(Clone, Serialize)]
struct ModProgress {
    phase: &'static str,
    message: Option<String>,
    done: u64,
    total: u64,
}

#[tauri::command]
async fn mod_download(
    window: WebviewWindow,
    root: Option<String>,
    id: String,
) -> Result<secular::View, String> {
    blocking(move || {
        with_operation(&window, |cache| {
            let release =
                selected_mod(&window, Some(&id))?.context("Сначала проверьте обновления")?;
            release.save(
                cache,
                |message| {
                    let _ = window.emit(
                        "mod-progress",
                        ModProgress {
                            phase: "download",
                            message: Some(message),
                            done: 0,
                            total: 0,
                        },
                    );
                },
                |done, total| {
                    let _ = window.emit(
                        "mod-progress",
                        ModProgress {
                            phase: "download",
                            message: None,
                            done,
                            total,
                        },
                    );
                },
            )?;
            secular::view(root.as_deref().map(Path::new), Some(&release), cache)
        })
    })
    .await
}

#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
enum ModInstall {
    Complete { view: secular::View },
    Confirm { game_version: Option<String> },
}

#[tauri::command]
async fn mod_install(
    window: WebviewWindow,
    root: String,
    id: String,
    allow_different_regulation: bool,
) -> Result<ModInstall, String> {
    blocking(move || {
        with_operation(&window, |cache| {
            let release =
                selected_mod(&window, Some(&id))?.context("Сначала проверьте обновления")?;
            let game = game::inspect(Path::new(&root))?;
            match secular::install(
                &game,
                &release,
                cache,
                allow_different_regulation,
                |done, total| {
                    let _ = window.emit(
                        "mod-progress",
                        ModProgress {
                            phase: "install",
                            message: None,
                            done: done as u64,
                            total: total as u64,
                        },
                    );
                },
            )? {
                secular::Outcome::Installed => Ok(ModInstall::Complete {
                    view: secular::view(Some(&game.root), Some(&release), cache)?,
                }),
                secular::Outcome::Confirm { game_version } => {
                    Ok(ModInstall::Confirm { game_version })
                }
            }
        })
    })
    .await
}

fn main() {
    tauri::Builder::default()
        .manage(AppOperation::default())
        .manage(LatestMod::default())
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event
                && window.state::<AppOperation>().0.try_lock().is_err()
            {
                api.prevent_close();
                let _ = window.emit("updater-busy", ());
            }
        })
        .invoke_handler(tauri::generate_handler![
            browse,
            verify,
            plan,
            locate,
            download_patch,
            install_patch,
            open_folder,
            mod_check,
            mod_status,
            mod_download,
            mod_install
        ])
        .run(tauri::generate_context!())
        .expect("failed to start Nightreign Launcher");
}
