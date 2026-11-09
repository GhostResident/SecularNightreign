pub const APP_TITLE: &str = "Секулярный Nightreign";
pub const APP_ID: &str = "SecularNightreign";
pub const APP_EXE: &str = "SecularNightreign.exe";
pub const GAME_EXE: &str = "nightreign.exe";
pub const GAME_PROCESS: &str = "nightreign.exe";
pub const SEAMLESS_EXE: &str = "nrsc_launcher.exe";
pub const SHORTCUT_NAME: &str = "Elden Ring Nightreign (секулярный мод).lnk";
pub const VERSION_URL: &str =
    "https://raw.githubusercontent.com/GhostResident/SecularNightreign/main/version.txt";
pub const PACKAGE_URL: &str =
    "https://github.com/GhostResident/SecularNightreign/archive/refs/heads/main.zip";
pub const VANILLA_REGULATION_SHA: &str =
    "876A3CA279A4561D0C69F81FE5E510C75C59AB9A8A201694F8CF4A42C91E0268";
pub const GAME_CHAIN: &[&str] = &["1.3.0", "1.3.1", "1.3.2", "1.3.3"];

pub struct ElAmigosPatch {
    pub to: &'static str,
    pub installer_name: &'static str,
    pub installer_url: &'static str,
    pub part_urls: [&'static str; 2],
}

pub const PATCH_131: ElAmigosPatch = ElAmigosPatch {
    to: "1.3.1",
    installer_name: "Elden Ring Nightreign update 1.03.0 - 1.03.1.exe",
    installer_url: "https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.0-1.03.1/Elden.Ring.Nightreign.update.1.03.0.-.1.03.1.exe",
    part_urls: [
        "https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.0-1.03.1/elamigos-1.bin.part1",
        "https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.0-1.03.1/elamigos-1.bin.part2",
    ],
};

pub const PATCH_132: ElAmigosPatch = ElAmigosPatch {
    to: "1.3.2",
    installer_name: "Elden Ring Nightreign update 1.03.1 - 1.03.2.exe",
    installer_url: "https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.1-1.03.2/Elden.Ring.Nightreign.update.1.03.1.-.1.03.2.exe",
    part_urls: [
        "https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.1-1.03.2/elamigos-1.bin.part1",
        "https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.1-1.03.2/elamigos-1.bin.part2",
    ],
};

pub const PATCH_133_URL: &str = "https://github.com/GhostResident/SecularNightreign/releases/download/game-patches/nightreign-patch-1.03.3.zip";
pub const PATCH_133_SHA256: &str =
    "1A35C1987EF23E38DAF5DB956824CD6A59B051B240CB7846BA9ECD981F5E7B98";
