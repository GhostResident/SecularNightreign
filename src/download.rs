use anyhow::{Context, Result, anyhow, bail};
use reqwest::{
    StatusCode,
    blocking::Client,
    header::{CONTENT_LENGTH, CONTENT_RANGE, ETAG, IF_RANGE, RANGE, USER_AGENT},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread,
    time::Duration,
};

use crate::events::Event;

const AGENT: &str = "Secular-Mod-Launcher/1.0 (+https://github.com/GhostResident)";

#[derive(Debug, Default, Serialize, Deserialize)]
struct PartialMeta {
    url: String,
    etag: Option<String>,
    total: Option<u64>,
}

fn meta_path(part: &Path) -> PathBuf {
    PathBuf::from(format!("{}.json", part.display()))
}

fn parse_total(content_range: &str) -> Option<u64> {
    content_range.rsplit('/').next()?.parse().ok()
}

pub fn client() -> Result<Client> {
    Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .context("не удалось создать HTTP-клиент")
}

fn async_client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .context("не удалось создать HTTP-клиент загрузки")
}

pub fn get_text(url: &str) -> Result<String> {
    let response = client()?
        .get(url)
        .header(USER_AGENT, AGENT)
        .send()
        .with_context(|| format!("не удалось открыть {url}"))?
        .error_for_status()
        .with_context(|| format!("сервер вернул ошибку для {url}"))?;
    Ok(response
        .text()?
        .trim_start_matches('\u{feff}')
        .trim()
        .to_owned())
}

/// Downloads to `<destination>.part` and only renames after a complete response.
/// The part and its ETag metadata intentionally survive cancellation and process exit.
pub fn download_resumable(
    url: &str,
    destination: &Path,
    expected_sha256: Option<&str>,
    cancel: &Arc<AtomicBool>,
    send: &impl Fn(Event),
) -> Result<()> {
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)?;
    }
    if destination.is_file() {
        if let Some(expected) = expected_sha256 {
            if crate::install::sha256_file(destination)?.eq_ignore_ascii_case(expected) {
                send(Event::Log(format!(
                    "Уже скачано и проверено: {}",
                    destination.display()
                )));
                return Ok(());
            }
            fs::remove_file(destination).ok();
        } else {
            send(Event::Log(format!(
                "Используется готовый файл: {}",
                destination.display()
            )));
            return Ok(());
        }
    }

    let part = PathBuf::from(format!("{}.part", destination.display()));
    let meta_file = meta_path(&part);
    let mut meta: PartialMeta = fs::read(&meta_file)
        .ok()
        .and_then(|v| serde_json::from_slice(&v).ok())
        .unwrap_or_default();
    if meta.url != url {
        fs::remove_file(&part).ok();
        fs::remove_file(&meta_file).ok();
        meta = PartialMeta {
            url: url.to_owned(),
            ..Default::default()
        };
    }

    let http = async_client()?;
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .context("не удалось запустить сетевой поток")?;
    let mut last_error = None;
    for attempt in 1..=5 {
        if cancel.load(Ordering::Relaxed) {
            bail!("операция отменена; загруженная часть сохранена");
        }
        let existing = part.metadata().map(|m| m.len()).unwrap_or(0);
        if existing > 0 {
            send(Event::Log(format!(
                "Продолжение загрузки с {:.1} МБ (попытка {attempt}/5)",
                existing as f64 / 1_048_576.0
            )));
        } else {
            send(Event::Log(format!(
                "Подключение к серверу (попытка {attempt}/5)…"
            )));
        }

        let mut request = http.get(url).header(USER_AGENT, AGENT);
        if existing > 0 {
            request = request.header(RANGE, format!("bytes={existing}-"));
            if let Some(etag) = &meta.etag {
                request = request.header(IF_RANGE, etag);
            }
        }
        let response = match runtime.block_on(tokio::time::timeout(
            Duration::from_secs(45),
            request.send(),
        )) {
            Ok(Ok(value)) => value,
            Ok(Err(error)) => {
                last_error = Some(anyhow!(error));
                thread::sleep(Duration::from_secs(attempt * 2));
                continue;
            }
            Err(_) => {
                last_error = Some(anyhow!("сервер не ответил за 45 секунд"));
                thread::sleep(Duration::from_secs(attempt * 2));
                continue;
            }
        };
        let status = response.status();
        if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
            if status == StatusCode::RANGE_NOT_SATISFIABLE {
                fs::remove_file(&part).ok();
                meta.total = None;
                continue;
            }
            bail!("сервер вернул HTTP {status} для {url}");
        }
        let resumed = existing > 0 && status == StatusCode::PARTIAL_CONTENT;
        let base = if resumed { existing } else { 0 };
        let total = response
            .headers()
            .get(CONTENT_RANGE)
            .and_then(|h| h.to_str().ok())
            .and_then(parse_total)
            .or_else(|| {
                response
                    .headers()
                    .get(CONTENT_LENGTH)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|v| v.parse::<u64>().ok())
                    .map(|v| v + base)
            });
        meta.etag = response
            .headers()
            .get(ETAG)
            .and_then(|h| h.to_str().ok())
            .map(ToOwned::to_owned)
            .or(meta.etag);
        meta.total = total.or(meta.total);
        fs::write(&meta_file, serde_json::to_vec_pretty(&meta)?)?;

        let mut output = OpenOptions::new()
            .create(true)
            .write(true)
            .append(resumed)
            .truncate(!resumed)
            .open(&part)?;
        let mut input = response;
        let mut done = base;
        let transfer = (|| -> Result<()> {
            loop {
                if cancel.load(Ordering::Relaxed) {
                    output.flush().ok();
                    bail!("операция отменена; загруженная часть сохранена");
                }
                let chunk = match runtime
                    .block_on(tokio::time::timeout(Duration::from_secs(90), input.chunk()))
                {
                    Ok(result) => result.context("соединение прервано")?,
                    Err(_) => bail!("за 90 секунд не получено данных"),
                };
                let Some(chunk) = chunk else {
                    break;
                };
                output.write_all(&chunk).context("ошибка записи на диск")?;
                done += chunk.len() as u64;
                send(Event::Progress {
                    done,
                    total: meta.total,
                });
            }
            output.flush()?;
            Ok(())
        })();
        if let Err(error) = transfer {
            if error.to_string().contains("операция отменена") {
                return Err(error);
            }
            send(Event::Log(format!("Сбой загрузки: {error}")));
            last_error = Some(error);
            thread::sleep(Duration::from_secs(attempt * 2));
            continue;
        }
        if let Some(total) = meta.total {
            let actual = part.metadata()?.len();
            if actual != total {
                last_error = Some(anyhow!("получено {actual} байт, ожидалось {total}"));
                thread::sleep(Duration::from_secs(attempt * 2));
                continue;
            }
        }
        if let Some(expected) = expected_sha256 {
            send(Event::Stage("Проверка SHA-256…".into()));
            let actual = crate::install::sha256_file(&part)?;
            if !actual.eq_ignore_ascii_case(expected) {
                fs::remove_file(&part).ok();
                bail!("контрольная сумма не совпала: {actual}");
            }
        }
        fs::rename(&part, destination)
            .or_else(|_| {
                fs::remove_file(destination).ok();
                fs::rename(&part, destination)
            })
            .with_context(|| format!("не удалось сохранить {}", destination.display()))?;
        fs::remove_file(meta_file).ok();
        send(Event::Progress {
            done: destination.metadata()?.len(),
            total: Some(destination.metadata()?.len()),
        });
        return Ok(());
    }
    Err(last_error.unwrap_or_else(|| anyhow!("загрузка не удалась после пяти попыток")))
}

pub fn concatenate(parts: &[PathBuf], destination: &Path, cancel: &Arc<AtomicBool>) -> Result<()> {
    let temporary = PathBuf::from(format!("{}.tmp", destination.display()));
    let mut output = File::create(&temporary)?;
    let mut buffer = vec![0_u8; 1024 * 1024];
    for part in parts {
        let mut input = File::open(part)?;
        loop {
            if cancel.load(Ordering::Relaxed) {
                bail!("операция отменена; скачанные части сохранены");
            }
            let count = input.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            output.write_all(&buffer[..count])?;
        }
    }
    output.flush()?;
    fs::rename(&temporary, destination)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::parse_total;

    #[test]
    fn parses_http_content_range() {
        assert_eq!(parse_total("bytes 1024-2047/8192"), Some(8192));
        assert_eq!(parse_total("bytes */123"), Some(123));
        assert_eq!(parse_total("broken"), None);
    }
}
