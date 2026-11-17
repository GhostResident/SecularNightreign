use std::{
    env,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    thread,
    time::Duration,
};

use anyhow::Result;
use zip::{ZipWriter, write::SimpleFileOptions};

use crate::{download, game, payload, updater::WorkDir};

#[test]
fn versions_and_game_layout() -> Result<()> {
    assert_eq!(game::normalize("1.03.2.0").as_deref(), Some("1.3.2"));
    assert_eq!(game::normalize("1, 3, 3, 0").as_deref(), Some("1.3.3"));
    let test = WorkDir::create(&env::temp_dir(), "Nightreign игра 'test'")?;
    let nested = test.path.join("Game");
    fs::create_dir(&nested)?;
    let exe = nested.join("nightreign.exe");
    fs::copy(env::current_exe()?, &exe)?;
    let scan = game::inspect(&test.path)?;
    assert_eq!(scan.version.as_deref(), Some(env!("CARGO_PKG_VERSION")));
    assert!(!scan.supported);
    let selected_game_dir = game::inspect(&nested)?;
    assert_eq!(selected_game_dir.root, scan.root);
    assert_eq!(selected_game_dir.game_dir, scan.game_dir);
    Ok(())
}

#[test]
fn wrapped_zip_overlays_game_files() -> Result<()> {
    let test = WorkDir::create(&env::temp_dir(), "nightreign-zip-test")?;
    let zipfile = test.path.join("hotfix.zip");
    let mut zip = ZipWriter::new(File::create(&zipfile)?);
    for (path, data) in [
        ("Hotfix/Game/nightreign.exe", "new exe"),
        ("Hotfix/Game/data/new.dat", "new data"),
    ] {
        zip.start_file(
            path,
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated),
        )?;
        zip.write_all(data.as_bytes())?;
    }
    zip.finish()?;
    let extracted = test.path.join("extracted");
    payload::extract(&zipfile, &extracted)?;
    let live = test.path.join("Game");
    fs::create_dir(&live)?;
    fs::write(live.join("nightreign.exe"), "old exe")?;
    fs::write(live.join("unchanged.dat"), "existing data")?;
    payload::apply(&payload::find_root(&extracted)?, &live, |_, _| {})?;
    assert_eq!(fs::read_to_string(live.join("nightreign.exe"))?, "new exe");
    assert_eq!(fs::read_to_string(live.join("data/new.dat"))?, "new data");
    assert_eq!(
        fs::read_to_string(live.join("unchanged.dat"))?,
        "existing data"
    );
    Ok(())
}

#[test]
fn copy_failure_restores_files_already_replaced() -> Result<()> {
    let test = WorkDir::create(&env::temp_dir(), "nightreign-rollback-test")?;
    let source = test.path.join("patch");
    let live = test.path.join("Game");
    fs::create_dir(&source)?;
    fs::create_dir(&live)?;
    for name in ["a.dat", "b.dat"] {
        fs::write(source.join(name), "replacement")?;
        fs::write(live.join(name), "original")?;
    }
    let error = payload::apply(&source, &live, |copied, _| {
        if copied == 1 {
            let pending = ["a.dat", "b.dat"]
                .into_iter()
                .map(|name| live.join(name))
                .find(|path| fs::read_to_string(path).unwrap() == "original")
                .unwrap();
            fs::remove_file(&pending).unwrap();
            fs::create_dir(pending).unwrap();
        }
    })
    .unwrap_err();
    assert!(error.to_string().contains("Замены отменены"));
    let targets = [live.join("a.dat"), live.join("b.dat")];
    assert!(
        targets
            .iter()
            .any(|path| fs::read_to_string(path).is_ok_and(|text| text == "original"))
    );
    assert!(
        !targets
            .iter()
            .any(|path| fs::read_to_string(path).is_ok_and(|text| text == "replacement"))
    );
    Ok(())
}

pub(crate) fn request(stream: &TcpStream) -> Result<String> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut request = String::new();
    let mut input = BufReader::new(stream);
    loop {
        let mut line = String::new();
        if input.read_line(&mut line)? == 0 || line == "\r\n" {
            return Ok(request.to_ascii_lowercase());
        }
        request.push_str(&line);
    }
}

#[test]
fn interrupted_download_resumes_with_range() -> Result<()> {
    let server = TcpListener::bind("127.0.0.1:0")?;
    let url = reqwest::Url::parse(&format!("http://{}/patch", server.local_addr()?))?;
    let serving = thread::spawn(move || -> Result<()> {
        let (mut first, _) = server.accept()?;
        assert!(!request(&first)?.contains("range:"));
        first.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\nETag: \"patch-1\"\r\nConnection: close\r\n\r\nabcde")?;
        first.shutdown(std::net::Shutdown::Write)?;
        let (mut second, _) = server.accept()?;
        let request = request(&second)?;
        assert!(request.contains("\r\nrange: bytes=5-\r\n"));
        assert!(request.contains("\r\nif-range: \"patch-1\"\r\n"));
        second.write_all(b"HTTP/1.1 206 Partial Content\r\nContent-Length: 5\r\nContent-Range: bytes 5-9/10\r\nETag: \"patch-1\"\r\nConnection: close\r\n\r\nfghij")?;
        Ok(())
    });
    let test = WorkDir::create(&env::temp_dir(), "nightreign-range-test")?;
    let client = reqwest::blocking::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(5))
        .build()?;
    let output = test.path.join(Path::new("patch.bin"));
    download::fetch(&client, &url, &output, |_| {}, |_, _| {})?;
    serving.join().expect("HTTP fixture failed")?;
    assert_eq!(fs::read(output)?, b"abcdefghij");
    Ok(())
}
