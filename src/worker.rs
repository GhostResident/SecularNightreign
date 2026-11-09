use anyhow::{Context, Result};
use crossbeam_channel::Sender;
use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use crate::{
    config::{PACKAGE_URL, VERSION_URL},
    download,
    events::Event,
    game_update, install, package, platform, seamless, state,
};

#[derive(Clone, Copy)]
pub enum Operation {
    Install,
    Launch,
    Uninstall,
    Check,
}

pub fn spawn(operation: Operation, game: PathBuf, tx: Sender<Event>, cancel: Arc<AtomicBool>) {
    std::thread::spawn(move || {
        let send = |event| {
            let _ = tx.send(event);
        };
        let result = match operation {
            Operation::Install => install_or_repair(&game, &cancel, &send, false),
            Operation::Launch => install_or_repair(&game, &cancel, &send, true).and_then(|_| {
                send(Event::Stage("Запуск игры…".into()));
                platform::launch_game(&game)?;
                Ok("Игра запущена".to_owned())
            }),
            Operation::Uninstall => {
                install::uninstall(&game, |s| send(Event::Log(s))).map(|close| {
                    if close {
                        "__CLOSE__".to_owned()
                    } else {
                        "Мод удалён".to_owned()
                    }
                })
            }
            Operation::Check => check(&game, &send),
        };
        match result {
            Ok(message) => {
                let close_after = message == "__CLOSE__";
                let text = if close_after {
                    "Мод удалён. Лаунчер закроется.".into()
                } else {
                    message
                };
                send(Event::Finished {
                    ok: true,
                    message: text,
                    close_after,
                });
            }
            Err(error) => send(Event::Finished {
                ok: false,
                message: format!("{error:#}"),
                close_after: false,
            }),
        }
    });
}

fn check(game: &Path, send: &impl Fn(Event)) -> Result<String> {
    install::require_game(game)?;
    let game_version = platform::game_version(game)?;
    send(Event::Log(format!("Версия игры: {game_version}")));
    send(Event::Stage("Проверка обновлений мода…".into()));
    let remote = download::get_text(VERSION_URL)?;
    let local = install::installed_version(game).unwrap_or_else(|| "не установлен".into());
    send(Event::Log(format!("Установленная версия мода: {local}")));
    send(Event::Log(format!("Версия мода на GitHub: {remote}")));
    Ok(if local == remote && game_version == "1.3.3" {
        "Игра и мод актуальны".into()
    } else {
        "Доступно обновление или восстановление".into()
    })
}

fn install_or_repair(
    game: &Path,
    cancel: &Arc<AtomicBool>,
    send: &impl Fn(Event),
    allow_offline_launch: bool,
) -> Result<String> {
    install::require_game(game)?;
    if platform::game_is_running() {
        anyhow::bail!("закройте игру перед обновлением мода");
    }
    seamless::install_if_present(game, |s| send(Event::Log(s)))?;
    let game_version = platform::game_version(game)?;
    if game_version != "1.3.3" {
        game_update::update_game(game, cancel, send)?;
    } else {
        send(Event::Log(
            "Игра уже версии 1.3.3; игровые патчи не требуются".into(),
        ));
    }

    send(Event::Stage("Проверка версии мода на GitHub…".into()));
    let local = install::installed_version(game);
    let local_package = game.join("SecularMod");
    let remote = match download::get_text(VERSION_URL) {
        Ok(version) => version,
        Err(error)
            if allow_offline_launch && local.is_some() && local_package.join("mod").is_dir() =>
        {
            send(Event::Log(format!("GitHub недоступен: {error:#}")));
            send(Event::Log(
                "Проверка и восстановление из локальной копии мода".into(),
            ));
            let installed =
                install::install_package(&local_package, game, |s| send(Event::Log(s)))?;
            return Ok(format!(
                "Сеть недоступна; локальная версия {installed} проверена"
            ));
        }
        Err(error)
            if allow_offline_launch
                && local.is_some()
                && install::installed_files_present(game) =>
        {
            send(Event::Log(format!("GitHub недоступен: {error:#}")));
            send(Event::Log(
                "Запуск с установленной версией мода без обновления".into(),
            ));
            return Ok("Сеть недоступна; используется установленная версия".into());
        }
        Err(error) => return Err(error).context("не удалось проверить версию мода"),
    };
    send(Event::Log(format!("GitHub: версия мода {remote}")));
    if let Some(local) = &local {
        send(Event::Log(format!("Установлено: версия мода {local}")));
    }

    if local.as_deref() == Some(remote.as_str()) && local_package.join("mod").is_dir() {
        send(Event::Stage(
            "Проверка и восстановление локальной версии…".into(),
        ));
        let installed = install::install_package(&local_package, game, |s| send(Event::Log(s)))?;
        return Ok(format!("Мод версии {installed} установлен и проверен"));
    }

    let zip = state::downloads_dir().join(format!("SecularNightreign-{remote}.zip"));
    if local.as_deref() != Some(remote.as_str()) || !zip.is_file() {
        send(Event::Stage("Скачивание мода…".into()));
        download::download_resumable(PACKAGE_URL, &zip, None, cancel, send)?;
    }
    if cancel.load(Ordering::Relaxed) {
        anyhow::bail!("операция отменена");
    }
    send(Event::Stage("Проверка и установка файлов мода…".into()));
    let extracted = package::extract_package(&zip).or_else(|first| {
        fs::remove_file(&zip).ok();
        send(Event::Log(format!(
            "Кэш ZIP повреждён ({first}); выполняется повторная загрузка"
        )));
        download::download_resumable(PACKAGE_URL, &zip, None, cancel, send)?;
        package::extract_package(&zip)
    })?;
    let installed = install::install_package(&extracted.root, game, |s| send(Event::Log(s)))?;
    if installed != remote {
        send(Event::Log(format!(
            "Предупреждение: в архиве версия {installed}, ожидалась {remote}"
        )));
    }
    Ok(format!("Мод версии {installed} установлен и проверен"))
}
