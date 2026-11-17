use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::Path,
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result, anyhow, bail, ensure};
use reqwest::{
    StatusCode, Url,
    blocking::{Client, ClientBuilder, Response},
    header::{ACCEPT_ENCODING, CONTENT_RANGE, ETAG, HeaderValue, IF_RANGE, LAST_MODIFIED, RANGE},
    redirect,
};

const RETRIES: u64 = 4;

pub fn client() -> Result<Client> {
    Ok(client_builder().build()?)
}

pub fn client_builder() -> ClientBuilder {
    Client::builder()
        .user_agent(concat!("NightreignLauncher/", env!("CARGO_PKG_VERSION")))
        .redirect(redirect::Policy::limited(5))
        .connect_timeout(Duration::from_secs(15))
        .timeout(Duration::from_secs(45))
}

pub fn fetch(
    client: &Client,
    url: &Url,
    output: &Path,
    on_log: impl Fn(String),
    on_bytes: impl Fn(u64, Option<u64>),
) -> Result<()> {
    let filename = output.file_name().context("Не задано имя загрузки")?;
    let mut name = filename.to_os_string();
    name.push(".partial");
    let partial = output.with_file_name(name);
    // Each preparation uses its own directory, so only this operation owns this partial.
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&partial)?;
    let mut validator = None;
    let mut last_error = None;
    for retry in 0..=RETRIES {
        let offset = fs::metadata(&partial)?.len();
        if retry > 0 {
            on_log(format!(
                "Повтор {retry}/{RETRIES}, сохранено {:.1} МиБ",
                offset as f64 / 1048576.0
            ));
            thread::sleep(Duration::from_secs(retry));
        }
        match attempt(client, url, &partial, offset, &mut validator, &on_bytes)? {
            Attempt::Done => {
                fs::rename(&partial, output).context("Не удалось сохранить загруженный файл")?;
                return Ok(());
            }
            Attempt::Retry(error) => last_error = Some(error),
        }
    }
    Err(last_error.unwrap_or_else(|| anyhow!("Загрузка прервана")))
}

enum Attempt {
    Done,
    Retry(anyhow::Error),
}

fn attempt(
    client: &Client,
    url: &Url,
    partial: &Path,
    offset: u64,
    validator: &mut Option<HeaderValue>,
    on_bytes: &impl Fn(u64, Option<u64>),
) -> Result<Attempt> {
    let mut request = client.get(url.clone()).header(ACCEPT_ENCODING, "identity");
    if offset > 0 {
        request = request.header(RANGE, format!("bytes={offset}-"));
        if let Some(value) = validator.as_ref() {
            request = request.header(IF_RANGE, value);
        }
    }
    let mut response = match request.send() {
        Ok(response) => response,
        Err(error) if error.is_redirect() => {
            return Err(error.without_url()).context("Не удалось перейти по перенаправлению");
        }
        Err(error) => return Ok(Attempt::Retry(error.without_url().into())),
    };

    let status = response.status();
    let (start, total) = match status {
        StatusCode::OK => {
            *validator = response_validator(&response);
            (0, response.content_length())
        }
        StatusCode::PARTIAL_CONTENT if offset > 0 => {
            let range = response
                .headers()
                .get(CONTENT_RANGE)
                .and_then(|value| value.to_str().ok())
                .and_then(parse_range);
            let expected = range.filter(|&(start, end, total)| {
                start == offset
                    && end < total
                    && response
                        .content_length()
                        .is_none_or(|n| n == end - start + 1)
            });
            let changed = validator
                .as_ref()
                .zip(response_validator(&response).as_ref())
                .is_some_and(|(old, new)| old != new);
            match expected {
                Some((_, _, total)) if !changed => (offset, Some(total)),
                _ => return restart(partial, "Сервер вернул несовместимый диапазон"),
            }
        }
        StatusCode::RANGE_NOT_SATISFIABLE if offset > 0 => {
            return restart(partial, "Сервер не принял диапазон");
        }
        StatusCode::REQUEST_TIMEOUT | StatusCode::TOO_MANY_REQUESTS => {
            return Ok(Attempt::Retry(anyhow!("HTTP {status}")));
        }
        other if other.is_server_error() => return Ok(Attempt::Retry(anyhow!("HTTP {other}"))),
        other => bail!("Сервер вернул HTTP {other}"),
    };

    let mut file = OpenOptions::new()
        .write(true)
        .truncate(start == 0)
        .append(start > 0)
        .open(partial)?;
    let mut got = start;
    let mut buffer = vec![0; 256 * 1024];
    let mut reported = Instant::now();
    on_bytes(got, total);
    loop {
        let count = match response.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) => return Ok(Attempt::Retry(anyhow!("Поток загрузки прервался: {error}"))),
        };
        file.write_all(&buffer[..count])
            .context("Ошибка записи загрузки")?;
        got += count as u64;
        if reported.elapsed() >= Duration::from_millis(150) {
            on_bytes(got, total);
            reported = Instant::now();
        }
    }
    if total.is_some_and(|total| got != total) {
        return Ok(Attempt::Retry(anyhow!(
            "Загружено {got} байт, получен неполный файл"
        )));
    }
    ensure!(got > 0, "Сервер вернул пустой файл");
    file.sync_all()?;
    on_bytes(got, Some(got));
    Ok(Attempt::Done)
}

fn restart(partial: &Path, reason: &str) -> Result<Attempt> {
    OpenOptions::new()
        .write(true)
        .truncate(true)
        .open(partial)?;
    Ok(Attempt::Retry(anyhow!("{reason}")))
}

fn response_validator(response: &Response) -> Option<HeaderValue> {
    response
        .headers()
        .get(ETAG)
        .filter(|value| value.as_bytes().starts_with(b"\""))
        .or_else(|| response.headers().get(LAST_MODIFIED))
        .cloned()
}

fn parse_range(header: &str) -> Option<(u64, u64, u64)> {
    let (positions, total) = header.strip_prefix("bytes ")?.split_once('/')?;
    let (start, end) = positions.split_once('-')?;
    let start = start.parse::<u64>().ok()?;
    let end = end.parse::<u64>().ok()?;
    (start <= end).then_some((start, end, total.parse().ok()?))
}
