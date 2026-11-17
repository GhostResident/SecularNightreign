use std::{
    fs::{self, File},
    io::{self, Cursor},
    path::Path,
};

use anyhow::{Context, Result, ensure};
use semver::Version;
use serde::{Deserialize, Serialize};
use zip::ZipArchive;

use crate::{
    game::{self, Game},
    mod_release::{Offer, Release, file_sha256, version_from_tag},
    payload::{self, metadata_if_present},
    platform,
    updater::WorkDir,
};

const FILES: [&str; 4] = [
    "regulation.bin",
    "dinput8.dll",
    "secular_text.bin",
    "secular_mute.txt",
];
const BACKUP: &str = "secular_backup";
const RECEIPT: &str = "launcher.json";
const SEAMLESS_ZIP: &[u8] = include_bytes!("../assets/Seamless.zip");
const VANILLA_1_3_3: &str = "876a3ca279a4561d0c69f81fe5e510c75c59ab9a8a201694f8cf4a42c91e0268";
// Identify files from the example manual installer when there is no Launcher receipt.
const LEGACY_MOD_HASHES: [&str; 2] = [
    "b23c26f6fd5f3921f95092a9f2032998e4a6fc5535f8275d76b65b9f6b852e2d",
    "d89ae0a6d9997a559e92778582862eb6e61624e98b3a2d5937fc95366d228f88",
];

#[derive(Deserialize, Serialize)]
struct Receipt {
    version: String,
    hashes: [String; FILES.len()],
    #[serde(default)]
    seamless_installed: bool,
}

struct Local {
    receipt: Option<Receipt>,
    hashes: [Option<String>; FILES.len()],
}

impl Local {
    fn read(dir: &Path) -> Result<Self> {
        let backup = dir.join(BACKUP);
        let receipt = if metadata_if_present(&backup)?.is_some() {
            game::ordinary_dir(&backup)?;
            let path = backup.join(RECEIPT);
            if let Some(meta) = metadata_if_present(&path)? {
                game::ordinary_file(&path)?;
                if meta.len() <= 4096 {
                    serde_json::from_slice::<Receipt>(&fs::read(path)?)
                        .ok()
                        .filter(|receipt| version_from_tag(&receipt.version).is_some())
                } else {
                    None
                }
            } else {
                None
            }
        } else {
            None
        };
        let mut hashes = [const { None }; FILES.len()];
        for (slot, name) in hashes.iter_mut().zip(FILES) {
            let file = dir.join(name);
            if metadata_if_present(&file)?.is_some() {
                *slot = Some(file_sha256(&file)?);
            }
        }
        Ok(Self { receipt, hashes })
    }

    fn version(&self) -> Option<Version> {
        version_from_tag(&self.receipt.as_ref()?.version)
    }

    fn intact(&self) -> bool {
        self.receipt.as_ref().is_some_and(|receipt| {
            self.hashes
                .iter()
                .zip(&receipt.hashes)
                .all(|(actual, expected)| actual.as_ref() == Some(expected))
        })
    }

    fn state(&self) -> &'static str {
        if self.intact() {
            "installed"
        } else if self.receipt.is_some() {
            "changed"
        } else if self.hashes[2].is_some()
            || self.hashes[3].is_some()
            || self.hashes[0].as_deref() == Some(LEGACY_MOD_HASHES[0])
            || self.hashes[1].as_deref() == Some(LEGACY_MOD_HASHES[1])
        {
            "manual"
        } else {
            "absent"
        }
    }

    fn seamless_installed(&self) -> bool {
        self.receipt
            .as_ref()
            .is_some_and(|receipt| receipt.seamless_installed)
    }

    fn complete(&self) -> bool {
        self.seamless_installed() && self.intact()
    }

    fn can_update(&self, release: &Release) -> bool {
        self.version().is_none_or(|version| {
            version < release.version || (version == release.version && !self.complete())
        })
    }

    fn is_mod_file(&self, index: usize, incoming: &str) -> bool {
        self.hashes[index].as_deref().is_some_and(|hash| {
            hash == incoming
                || hash == LEGACY_MOD_HASHES[index]
                || self
                    .receipt
                    .as_ref()
                    .is_some_and(|receipt| receipt.hashes[index] == hash)
        })
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    installed: Option<String>,
    state: &'static str,
    latest: Option<Offer>,
    needs_install: bool,
    newer_installed: bool,
}

pub fn view(root: Option<&Path>, release: Option<&Release>, cache: &Path) -> Result<View> {
    let local = root
        .map(|root| {
            let game = game::inspect(root)?;
            Local::read(&game.game_dir)
        })
        .transpose()?;
    Ok(View {
        state: local.as_ref().map_or("unselected", Local::state),
        needs_install: release
            .is_some_and(|release| local.as_ref().is_none_or(|local| local.can_update(release))),
        newer_installed: release
            .zip(local.as_ref().and_then(Local::version))
            .is_some_and(|(latest, installed)| installed > latest.version),
        installed: local.and_then(|local| local.receipt.map(|receipt| receipt.version)),
        latest: release.map(|release| release.offer(cache)),
    })
}

pub enum Outcome {
    Installed,
    Confirm { game_version: Option<String> },
}

pub fn install(
    game: &Game,
    release: &Release,
    cache: &Path,
    allow_different_regulation: bool,
    progress: impl Fn(usize, usize),
) -> Result<Outcome> {
    let zip = release.cached_zip(cache)?;
    install_zip(
        game,
        &zip,
        &release.tag,
        cache,
        allow_different_regulation,
        SEAMLESS_ZIP,
        progress,
    )
}

fn install_zip(
    game: &Game,
    zip: &Path,
    tag: &str,
    cache: &Path,
    allow_different_regulation: bool,
    seamless_zip: &[u8],
    progress: impl Fn(usize, usize),
) -> Result<Outcome> {
    ensure!(
        !platform::nightreign_running()?,
        "Закройте Nightreign перед установкой мода"
    );
    let dir = &game.game_dir;
    let local = Local::read(dir)?;
    let version = version_from_tag(tag).context("Не удалось прочитать версию мода")?;
    if let Some(installed) = local.version() {
        ensure!(
            installed <= version,
            "Установленная версия мода {installed} новее {tag}"
        );
        if installed == version && local.complete() {
            return Ok(Outcome::Installed);
        }
    }
    let current_regulation = local.hashes[0]
        .as_deref()
        .context("В папке игры нет regulation.bin")?;
    let work = WorkDir::create(cache, "secular-install")?;
    let secular = work.path.join("secular");
    fs::create_dir(&secular)?;
    let hashes = stage_files(zip, &secular)?;
    let already_modded = local.is_mod_file(0, &hashes[0]);
    let backup = dir.join(BACKUP);
    let mut existing_backups = [false; 2];
    for (exists, name) in existing_backups.iter_mut().zip(FILES) {
        let path = backup.join(name);
        *exists = metadata_if_present(&path)?.is_some();
        if *exists {
            game::ordinary_file(&path)?;
        }
    }
    if !allow_different_regulation
        && !already_modded
        && !existing_backups[0]
        && current_regulation != VANILLA_1_3_3
    {
        return Ok(Outcome::Confirm {
            game_version: game.version.clone(),
        });
    }

    let seamless = work.path.join("seamless");
    let mut packages = Vec::with_capacity(2);
    if !local.seamless_installed() {
        payload::extract_zip(Cursor::new(seamless_zip), &seamless)
            .context("Не удалось распаковать Seamless.zip")?;
        ensure!(
            metadata_if_present(&seamless.join(BACKUP))?.is_none(),
            "Seamless.zip содержит папку резервных копий SecularNightreign"
        );
        packages.push(seamless.as_path());
    }

    let staged_backup = secular.join(BACKUP);
    fs::create_dir(&staged_backup)?;
    for (index, exists) in existing_backups.into_iter().enumerate() {
        if !exists && local.hashes[index].is_some() && !local.is_mod_file(index, &hashes[index]) {
            let name = FILES[index];
            fs::copy(dir.join(name), staged_backup.join(name))
                .with_context(|| format!("Не удалось подготовить копию {name}"))?;
        }
    }
    let receipt = Receipt {
        version: tag.to_owned(),
        hashes,
        seamless_installed: true,
    };
    let record = File::create_new(staged_backup.join(RECEIPT))?;
    serde_json::to_writer(&record, &receipt)?;
    record.sync_all()?;
    drop(record);
    ensure!(
        !platform::nightreign_running()?,
        "Закройте Nightreign перед установкой мода"
    );
    packages.push(secular.as_path());
    payload::apply_in_order(&packages, dir, progress)?;
    Ok(Outcome::Installed)
}

fn stage_files(zip: &Path, destination: &Path) -> Result<[String; FILES.len()]> {
    let mut archive = ZipArchive::new(File::open(zip)?).context("Не удалось открыть ZIP мода")?;
    let prefix = if archive
        .file_names()
        .any(|name| name == "SecularNightreign/files/regulation.bin")
    {
        "SecularNightreign/files"
    } else {
        "files"
    };
    let mut hashes = [const { String::new() }; FILES.len()];
    for (name, hash) in FILES.into_iter().zip(&mut hashes) {
        let path = format!("{prefix}/{name}");
        let mut entry = archive
            .by_name(&path)
            .with_context(|| format!("В ZIP нет {path}"))?;
        ensure!(
            !entry.is_dir() && !entry.is_symlink() && entry.size() > 0,
            "В ZIP ожидается файл {path}"
        );
        let staged = destination.join(name);
        let mut file = File::create_new(&staged)?;
        let written = io::copy(&mut entry, &mut file).with_context(|| path.clone())?;
        ensure!(written == entry.size(), "Неполный файл {path}");
        file.sync_all()?;
        drop(file);
        *hash = file_sha256(&staged)?;
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use std::{env, io::Write};
    use zip::{ZipWriter, write::SimpleFileOptions};

    use super::*;

    fn package(path: &Path, prefix: &str, marker: &str) -> Result<()> {
        let mut zip = ZipWriter::new(File::create(path)?);
        for name in FILES {
            zip.start_file(format!("{prefix}/{name}"), SimpleFileOptions::default())?;
            write!(zip, "{marker} {name}")?;
        }
        zip.start_file(
            "SecularNightreign/scripts/install.ps1",
            SimpleFileOptions::default(),
        )?;
        zip.write_all(b"test script excluded from runtime payload")?;
        zip.finish()?;
        Ok(())
    }

    fn test_seamless_zip() -> Result<Vec<u8>> {
        let mut zip = ZipWriter::new(Cursor::new(Vec::new()));
        for (name, data) in [
            ("dinput8.dll", "coop loader"),
            ("coop.cfg", "new coop config"),
        ] {
            zip.start_file(name, SimpleFileOptions::default())?;
            zip.write_all(data.as_bytes())?;
        }
        Ok(zip.finish()?.into_inner())
    }

    #[test]
    fn installs_and_updates_without_replacing_original_backups() -> Result<()> {
        let fixture = WorkDir::create(&env::temp_dir(), "secular-install-qa")?;
        let dir = fixture.path.join("Test game").join("Game");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("nightreign.exe"), b"fixture")?;
        fs::write(dir.join("regulation.bin"), "vanilla regulation")?;
        fs::write(dir.join("dinput8.dll"), "previous DLL")?;
        fs::write(dir.join("coop.cfg"), "old coop config")?;
        fs::write(dir.join("secular_config.ini"), "fps_limit=144")?;
        let game = game::inspect(&dir)?;
        let zip = fixture.path.join("mod.zip");
        package(&zip, "SecularNightreign/files", "v1")?;
        let first = "sm-1";
        let second = "sm-2";
        let seamless = test_seamless_zip()?;
        assert!(matches!(
            install_zip(
                &game,
                &zip,
                first,
                &fixture.path,
                false,
                &seamless,
                |_, _| {}
            )?,
            Outcome::Confirm { .. }
        ));
        assert!(!dir.join(BACKUP).exists());
        assert_eq!(
            fs::read_to_string(dir.join("regulation.bin"))?,
            "vanilla regulation"
        );

        install_zip(
            &game,
            &zip,
            first,
            &fixture.path,
            true,
            &seamless,
            |done, _| {
                if done == 2 {
                    assert_eq!(
                        fs::read_to_string(dir.join("dinput8.dll")).unwrap(),
                        "coop loader"
                    );
                    assert_eq!(
                        fs::read_to_string(dir.join("regulation.bin")).unwrap(),
                        "vanilla regulation"
                    );
                }
            },
        )?;
        let installed = Local::read(&dir)?;
        assert_eq!(installed.version(), Some(Version::new(1, 0, 0)));
        assert_eq!(
            view(Some(&dir), None, &fixture.path)?.installed,
            Some(first.to_owned())
        );
        assert!(installed.complete());
        assert_eq!(fs::read_to_string(dir.join("coop.cfg"))?, "new coop config");

        let receipt_path = dir.join(BACKUP).join(RECEIPT);
        let mut old_receipt: serde_json::Value = serde_json::from_slice(&fs::read(&receipt_path)?)?;
        old_receipt
            .as_object_mut()
            .unwrap()
            .remove("seamless_installed");
        old_receipt["version"] = serde_json::Value::String("1.0.0".to_owned());
        fs::write(&receipt_path, serde_json::to_vec(&old_receipt)?)?;
        assert!(!Local::read(&dir)?.complete());
        fs::write(dir.join("coop.cfg"), "old again")?;
        install_zip(
            &game,
            &zip,
            first,
            &fixture.path,
            false,
            &seamless,
            |_, _| {},
        )?;
        assert_eq!(fs::read_to_string(dir.join("coop.cfg"))?, "new coop config");

        package(&zip, "files", "v2")?;
        fs::write(dir.join("coop.cfg"), "player's coop config")?;
        install_zip(
            &game,
            &zip,
            second,
            &fixture.path,
            false,
            b"must be skipped on update",
            |_, _| {},
        )?;
        let local = Local::read(&dir)?;
        assert_eq!(local.version(), Some(Version::new(2, 0, 0)));
        assert!(local.intact());
        assert_eq!(
            view(Some(&dir), None, &fixture.path)?.installed,
            Some(second.to_owned())
        );
        assert_eq!(
            fs::read_to_string(dir.join("coop.cfg"))?,
            "player's coop config"
        );
        for name in FILES {
            assert_eq!(fs::read_to_string(dir.join(name))?, format!("v2 {name}"));
        }
        let backup = dir.join(BACKUP);
        assert_eq!(
            fs::read_to_string(backup.join("regulation.bin"))?,
            "vanilla regulation"
        );
        assert_eq!(
            fs::read_to_string(backup.join("dinput8.dll"))?,
            "previous DLL"
        );
        assert_eq!(
            fs::read_to_string(dir.join("secular_config.ini"))?,
            "fps_limit=144"
        );
        assert!(!dir.join("scripts").exists());
        assert!(
            install_zip(
                &game,
                &zip,
                first,
                &fixture.path,
                true,
                &seamless,
                |_, _| {}
            )
            .is_err()
        );

        fs::write(dir.join("regulation.bin"), b"overwritten by game update")?;
        assert_eq!(Local::read(&dir)?.state(), "changed");
        Ok(())
    }

    #[test]
    fn recognizes_manual_installation() -> Result<()> {
        let fixture = WorkDir::create(&env::temp_dir(), "secular-manual-qa")?;
        fs::write(fixture.path.join("secular_text.bin"), b"legacy files")?;
        let local = Local::read(&fixture.path)?;
        assert_eq!(local.state(), "manual");
        assert!(local.version().is_none());
        Ok(())
    }

    #[test]
    #[ignore = "QA archive supplied by user"]
    fn installs_the_supplied_archive_in_a_fixture() -> Result<()> {
        let zip = std::path::PathBuf::from(env::var_os("SECULAR_QA_ARCHIVE").unwrap());
        let fixture = WorkDir::create(&env::temp_dir(), "secular-sample-qa")?;
        let dir = fixture.path.join("fixture").join("Game");
        fs::create_dir_all(&dir)?;
        fs::write(dir.join("nightreign.exe"), b"fixture")?;
        fs::write(dir.join("regulation.bin"), b"fixture original")?;
        let game = game::inspect(&dir)?;
        let version = "v1.0.0";
        assert!(matches!(
            install_zip(
                &game,
                &zip,
                version,
                &fixture.path,
                true,
                &test_seamless_zip()?,
                |_, _| {}
            )?,
            Outcome::Installed
        ));
        let installed = Local::read(&dir)?;
        assert!(installed.intact());
        assert_eq!(installed.version(), Some(Version::new(1, 0, 0)));
        assert_eq!(installed.hashes[0].as_deref(), Some(LEGACY_MOD_HASHES[0]));
        assert_eq!(installed.hashes[1].as_deref(), Some(LEGACY_MOD_HASHES[1]));
        assert_eq!(
            fs::read(dir.join(BACKUP).join("regulation.bin"))?,
            b"fixture original"
        );
        assert!(!dir.join("scripts").exists());
        Ok(())
    }
}
