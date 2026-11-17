use std::{
    fs::{self, File},
    io::Read,
    path::{Path, PathBuf},
};

use anyhow::{Context, Result, bail, ensure};
use reqwest::{StatusCode, Url, blocking::Client, header};
use semver::Version;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::{download, game, payload, updater::WorkDir};

pub const NAME: &str = "SecularNightreign";
const ZIP: &str = "SecularNightreign.zip";
const RELEASES_API: &str = "https://api.github.com/repos/GhostResident/SecularNightreign/releases";
const RELEASES_SITE: &str = "https://github.com/GhostResident/SecularNightreign/releases";

#[derive(Deserialize)]
struct GitHubRelease {
    tag_name: String,
    draft: bool,
    prerelease: bool,
    assets: Vec<GitHubAsset>,
}

#[derive(Deserialize)]
struct GitHubAsset {
    id: u64,
    name: String,
    size: u64,
    state: String,
    digest: Option<String>,
    browser_download_url: String,
}

#[derive(Clone)]
pub struct Release {
    pub id: u64,
    pub tag: String,
    pub version: Version,
    size: u64,
    digest: Option<String>,
    url: Url,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Offer {
    id: String,
    version: String,
    size: u64,
    downloaded: bool,
}

pub fn latest() -> Result<Option<Release>> {
    let client = download::client_builder().https_only(true).build()?;
    read_releases(&client, &Url::parse(RELEASES_API)?)
}

fn read_releases(client: &Client, api: &Url) -> Result<Option<Release>> {
    let mut best: Option<Release> = None;
    let mut page = 1_u32;
    loop {
        let response = client
            .get(api.clone())
            .query(&[("per_page", 100), ("page", page)])
            .header(header::ACCEPT, "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2026-03-10")
            .send()
            .context("Не удалось проверить релизы SecularNightreign")?;
        match response.status() {
            StatusCode::OK => {}
            StatusCode::NOT_FOUND => {
                bail!("GitHub не нашёл публичный репозиторий GhostResident/SecularNightreign")
            }
            StatusCode::FORBIDDEN | StatusCode::TOO_MANY_REQUESTS => {
                bail!("GitHub ограничил доступ к проверке релизов. Повторите позже.");
            }
            code => bail!("GitHub вернул HTTP {code}"),
        }
        let has_next = response
            .headers()
            .get(header::LINK)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|links| links.contains("rel=\"next\""));
        let releases: Vec<GitHubRelease> = serde_json::from_reader(response)
            .context("Не удалось прочитать список релизов GitHub")?;
        for release in releases.into_iter().filter_map(release_with_zip) {
            if best
                .as_ref()
                .is_none_or(|latest| latest.version < release.version)
            {
                best = Some(release);
            }
        }
        if !has_next {
            return Ok(best);
        }
        page += 1;
    }
}

pub fn version_from_tag(tag: &str) -> Option<Version> {
    let version = if let Some(number) = tag.strip_prefix("sm-") {
        Version::new(number.parse().ok()?, 0, 0)
    } else {
        let version = tag
            .strip_prefix("mod-v")
            .or_else(|| tag.strip_prefix('v'))
            .unwrap_or(tag);
        Version::parse(version).ok()?
    };
    (version.pre.is_empty() && version.build.is_empty()).then_some(version)
}

fn release_with_zip(release: GitHubRelease) -> Option<Release> {
    if release.draft || release.prerelease {
        return None;
    }
    let tag = release.tag_name;
    let version = version_from_tag(&tag)?;
    let asset = release.assets.into_iter().find(|asset| {
        asset.name == ZIP && asset.state == "uploaded" && asset.id > 0 && asset.size > 0
    })?;
    let expected = format!("{RELEASES_SITE}/download/{tag}/{ZIP}");
    if !asset.browser_download_url.eq_ignore_ascii_case(&expected) {
        return None;
    }
    Some(Release {
        id: asset.id,
        tag,
        version,
        size: asset.size,
        digest: asset.digest,
        url: Url::parse(&asset.browser_download_url).ok()?,
    })
}

impl Release {
    pub fn offer(&self, cache: &Path) -> Offer {
        Offer {
            id: self.id.to_string(),
            version: self.tag.clone(),
            size: self.size,
            downloaded: self.cached_zip(cache).is_ok(),
        }
    }

    pub fn cached_zip(&self, cache: &Path) -> Result<PathBuf> {
        let folder = self.asset_folder(cache);
        game::ordinary_dir(cache)?;
        game::ordinary_dir(&cache.join(NAME))?;
        game::ordinary_dir(&folder)?;
        let path = folder.join(ZIP);
        self.verify(&path)
            .context("Скачайте архив выбранного релиза")?;
        Ok(path)
    }

    fn asset_folder(&self, cache: &Path) -> PathBuf {
        cache.join(NAME).join(self.id.to_string())
    }

    pub fn save(
        &self,
        cache: &Path,
        log: impl Fn(String),
        progress: impl Fn(u64, u64),
    ) -> Result<()> {
        if self.cached_zip(cache).is_ok() {
            return Ok(());
        }
        let client = download::client_builder().https_only(true).build()?;
        self.fetch(&client, cache, log, progress)
    }

    fn fetch(
        &self,
        client: &Client,
        cache: &Path,
        log: impl Fn(String),
        progress: impl Fn(u64, u64),
    ) -> Result<()> {
        let mod_cache = cache.join(NAME);
        fs::create_dir_all(&mod_cache)?;
        game::ordinary_dir(&mod_cache)?;
        let folder = self.asset_folder(cache);
        fs::create_dir_all(&folder)?;
        game::ordinary_dir(&folder)?;
        let temporary = WorkDir::create(&folder, "download")?;
        let zip = temporary.path.join(ZIP);
        download::fetch(client, &self.url, &zip, log, |got, _| {
            progress(got, self.size)
        })?;
        self.verify(&zip)?;
        fs::rename(&zip, folder.join(ZIP)).context("Не удалось сохранить SecularNightreign.zip")?;
        Ok(())
    }

    fn verify(&self, path: &Path) -> Result<()> {
        ensure!(
            game::ordinary_file(path)?.len() == self.size,
            "Размер ZIP расходится с релизом"
        );
        ensure!(
            payload::archive_extension(path)? == "zip",
            "Релиз должен содержать ZIP"
        );
        if let Some(expected) = &self.digest {
            let actual = file_sha256(path)?;
            ensure!(
                expected.eq_ignore_ascii_case(&format!("sha256:{actual}")),
                "SHA-256 архива расходится с релизом GitHub"
            );
        }
        Ok(())
    }
}

pub fn file_sha256(path: &Path) -> Result<String> {
    game::ordinary_file(path)?;
    let mut file = File::open(path)?;
    let mut hash = Sha256::new();
    let mut buffer = [0; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hash.update(&buffer[..read]);
    }
    Ok(format!("{:x}", hash.finalize()))
}

#[cfg(test)]
mod tests {
    use std::{
        env,
        io::{Cursor, Write},
        net::TcpListener,
        thread,
        time::Duration,
    };

    use serde_json::json;
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;
    use crate::tests::request;

    #[test]
    fn accepts_numbered_sm_releases() {
        let tag = "sm-1";
        let selected = release_with_zip(GitHubRelease {
            tag_name: tag.to_owned(),
            draft: false,
            prerelease: false,
            assets: vec![GitHubAsset {
                id: 42,
                name: ZIP.to_owned(),
                size: 1024,
                state: "uploaded".to_owned(),
                digest: None,
                browser_download_url: format!("{RELEASES_SITE}/download/{tag}/{ZIP}"),
            }],
        })
        .expect("sm-1 should be offered");
        assert_eq!(selected.tag, tag);
        assert_eq!(selected.version, Version::new(1, 0, 0));
        assert!(version_from_tag("sm-10") > version_from_tag("sm-2"));
    }

    #[test]
    fn selects_newest_mod_release_and_validates_download() -> Result<()> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        zip.start_file(
            "SecularNightreign/files/regulation.bin",
            SimpleFileOptions::default(),
        )?;
        zip.write_all(b"test release")?;
        let zip = zip.finish()?.into_inner();
        let asset = |tag: &str, name: &str| {
            json!({
                "id": 537,
                "name": name,
                "size": zip.len(),
                "state": "uploaded",
                "digest": format!("sha256:{:x}", Sha256::digest(&zip)),
                "browser_download_url": format!("{RELEASES_SITE}/download/{tag}/{name}")
            })
        };
        let release = |tag: &str, name: &str, pre: bool| json!({ "tag_name": tag, "draft": false, "prerelease": pre, "assets": [asset(tag, name)] });
        let page1 = json!([
            release("v8.0.0", "Launcher.exe", false),
            release("v3.0.0", ZIP, true),
            release("v1.9.0", ZIP, false)
        ])
        .to_string();
        let page2 = json!([release("v1.10.0", ZIP, false)]).to_string();
        let server = TcpListener::bind("127.0.0.1:0")?;
        let origin = format!("http://{}", server.local_addr()?);
        let expected = zip.clone();
        let fixture = thread::spawn(move || -> Result<()> {
            for (query, headers, body) in [
                (
                    "page=1 ",
                    "Link: <https://api.github.com/next>; rel=\"next\"\r\n",
                    page1.as_bytes(),
                ),
                ("page=2 ", "", page2.as_bytes()),
                ("get /asset ", "", zip.as_slice()),
            ] {
                let (mut stream, _) = server.accept()?;
                assert!(request(&stream)?.lines().next().unwrap().contains(query));
                write!(
                    stream,
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n{headers}\r\n",
                    body.len()
                )?;
                stream.write_all(body)?;
            }
            Ok(())
        });
        let client = download::client_builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()?;
        let mut newest =
            read_releases(&client, &Url::parse(&format!("{origin}/releases"))?)?.unwrap();
        assert_eq!(newest.version, Version::new(1, 10, 0));
        newest.url = Url::parse(&format!("{origin}/asset"))?;
        let cache = WorkDir::create(&env::temp_dir(), "secular-test-release")?;
        newest.fetch(&client, &cache.path, |_| {}, |_, _| {})?;
        fixture.join().expect("release server failed")?;
        let cached = newest.cached_zip(&cache.path)?;
        assert_eq!(fs::read(&cached)?, expected);
        let mut damaged = expected;
        *damaged.last_mut().unwrap() ^= 1;
        fs::write(&cached, damaged)?;
        assert!(newest.cached_zip(&cache.path).is_err());
        Ok(())
    }
}
