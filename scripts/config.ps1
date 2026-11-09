# Откуда берутся обновления мода: репозиторий GitHub и ветка.
$Repo   = 'GhostResident/SecularNightreign'
$Branch = 'main'
$ShortcutName = 'Elden Ring Nightreign (секулярный мод)'

# Патчи игры для обновления до 1.03.3, под которую собран мод. Менять не обязательно:
# 1.3.1 и 1.3.2 качаются с GitHub, 1.3.3 лежит в packages установщика (Zip — запасная ссылка).
$PatchUrls = @{
  '1.3.1' = @{
    Exe   = 'https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.0-1.03.1/Elden.Ring.Nightreign.update.1.03.0.-.1.03.1.exe'
    Part1 = 'https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.0-1.03.1/elamigos-1.bin.part1'
    Part2 = 'https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.0-1.03.1/elamigos-1.bin.part2'
  }
  '1.3.2' = @{
    Exe   = 'https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.1-1.03.2/Elden.Ring.Nightreign.update.1.03.1.-.1.03.2.exe'
    Part1 = 'https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.1-1.03.2/elamigos-1.bin.part1'
    Part2 = 'https://github.com/Elixir-Raj/nightreign-updates/releases/download/v1.03.1-1.03.2/elamigos-1.bin.part2'
  }
  '1.3.3' = @{ Zip = 'https://github.com/GhostResident/SecularNightreign/releases/download/game-patches/nightreign-patch-1.03.3.zip' }
}

$RawUrl = "https://raw.githubusercontent.com/$Repo/$Branch"
$ZipUrl = "https://github.com/$Repo/archive/refs/heads/$Branch.zip"
