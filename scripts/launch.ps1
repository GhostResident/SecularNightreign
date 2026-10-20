# Ярлык на рабочем столе запускает этот скрипт (окно скрыто):
#   1) если на GitHub вышла новая версия мода — скачивает и ставит её;
#   2) проверяет, что мод на месте (после обновления или проверки файлов игры они возвращаются к оригиналу), и чинит;
#   3) запускает игру: через Seamless Co-op, если он есть, иначе напрямую.
$ErrorActionPreference = 'Stop'
$pkg  = Split-Path $PSScriptRoot
$game = Split-Path $pkg
. (Join-Path $PSScriptRoot 'common.ps1')
Add-Type -AssemblyName System.Windows.Forms
$log = Join-Path $pkg 'launch.log'
if((Test-Path $log) -and (Get-Item $log).Length -gt 200KB){ [IO.File]::Delete($log) }
function Log($s){ Add-Content $log ("{0:yyyy-MM-dd HH:mm:ss} {1}" -f (Get-Date), $s) -Encoding UTF8 }
function Say($text, $icon = 'Information'){ [void][Windows.Forms.MessageBox]::Show($text, 'Секулярный Nightreign', 'OK', $icon) }

if(Test-GameRunning){ Say 'Игра уже запущена.'; exit 0 }

$local = [version]((Get-Content (Join-Path $pkg 'version.txt') -Encoding UTF8 | Select-Object -First 1).Trim())
$updated = $false
$new = $null
try {
  $remote = Get-RemoteVersion
  Log "версия: установлена $local, на GitHub $remote"
  if($remote -gt $local){
    $new = Get-ModPackage
    Log "скачано обновление $remote"
  }
} catch {
  # нет сети или GitHub недоступен: играем на установленной версии
  Log "обновление не проверено: $($_.Exception.Message)"
}

try {
  if($new){
    try { & (Join-Path $new 'scripts\install.ps1') -Game $game -Auto *>&1 | ForEach-Object { Log "  $_" } }
    finally { Remove-ModPackage $new }
    $updated = $true
    Log "обновлено до $remote"
  } else {
    & (Join-Path $PSScriptRoot 'install.ps1') -Game $game -Auto *>&1 | ForEach-Object { Log "  $_" }
  }
} catch {
  Log "ошибка установки: $($_.Exception.Message)"
  Say ("Мод не удалось установить:`n`n" + $_.Exception.Message + "`n`nИгра не запущена. Удалите мод («Удалить мод.bat» в папке игры, SecularMod) или установите его заново.") 'Error'
  exit 1
}

$sc = Join-Path $game 'nrsc_launcher.exe'
$exe = if(Test-Path $sc){ $sc } else { Join-Path $game 'nightreign.exe' }
Log "запуск: $exe"
Start-Process -FilePath $exe -WorkingDirectory $game
