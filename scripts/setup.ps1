# Установщик для игроков: обновляет игру до 1.03.3 (мод собран под неё), ставит
# Seamless Co-op из Seamless.zip (если он лежит рядом), скачивает с GitHub последнюю
# версию мода и устанавливает её. Совместим с Windows PowerShell 5.1.
$ErrorActionPreference = 'Stop'
$here = Split-Path $PSScriptRoot
. (Join-Path $PSScriptRoot 'common.ps1')
try {
  $game = Find-Game
  if(-not $game){ throw 'Папка с игрой не выбрана. Установка отменена.' }
  Write-Host "Папка игры: $game"
  if(Test-GameRunning){ throw 'Закройте игру перед установкой.' }

  # мод собран под 1.03.3: если игра старее — обновляем цепочкой патчей
  Update-Game $game

  $sz = Join-Path $here 'Seamless.zip'
  if(Test-Path $sz){ Install-Seamless $sz $game }

  # установщик лежит в самом пакете мода (скачан репозиторий целиком) — ставим его, иначе скачиваем с GitHub
  if(Test-Path (Join-Path $here 'version.txt')){ $pkg = $here }
  else {
    Write-Host 'Скачиваю мод с GitHub...'
    $pkg = Get-ModPackage
  }
  try { & (Join-Path $pkg 'scripts\install.ps1') -Game $game }
  finally { if($pkg -ne $here){ Remove-ModPackage $pkg } }
} catch {
  Write-Host ''
  Write-Host ('ОШИБКА: ' + $_.Exception.Message)
  exit 1
}
