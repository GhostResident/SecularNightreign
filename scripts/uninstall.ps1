# Удаление мода «Секулярный Nightreign»: возвращает оригинальные файлы из secular_backup, убирает ярлык и папку SecularMod.
# Seamless Co-op остаётся.
$ErrorActionPreference = 'Stop'
. (Join-Path $PSScriptRoot 'common.ps1')
try {
  $game = Find-Game
  if(-not $game){ throw 'Папка с игрой не выбрана.' }
  Write-Host "Папка игры: $game"
  if(Test-GameRunning){ throw 'Закройте игру перед удалением мода.' }

  $backup = Join-Path $game 'secular_backup'
  foreach($n in 'dinput8.dll','secular_text.bin','secular_mute.txt','secular_config.ini','secular_text.log'){
    $p = Join-Path $game $n
    if(Test-Path $p){ [IO.File]::Delete($p); Write-Host "удалён $n" }
  }
  if(Test-Path $backup){
    foreach($n in 'regulation.bin','dinput8.dll','steam_emu.ini','OnlineFix.ini'){
      $b = Join-Path $backup $n
      if(Test-Path $b){ Copy-Item $b (Join-Path $game $n) -Force; Write-Host "восстановлен $n" }
    }
    if(-not (Test-Path (Join-Path $backup 'regulation.bin'))){
      Write-Host 'Резервной копии regulation.bin нет. Если механики мода остались, проверьте целостность файлов игры в Steam.'
    }
    [IO.Directory]::Delete($backup, $true)
  }
  Remove-DesktopShortcut
  $home_ = Join-Path $game 'SecularMod'
  if(Test-Path $home_){
    if([IO.Path]::GetFullPath($PSScriptRoot).StartsWith([IO.Path]::GetFullPath($home_), 'OrdinalIgnoreCase')){
      # удаление запущено из самой папки SecularMod: стереть её после выхода
      Start-Process cmd.exe -ArgumentList "/c timeout /t 3 >nul & rd /s /q `"$home_`"" -WindowStyle Hidden
    } else { [IO.Directory]::Delete($home_, $true) }
  }
  if(Test-Path $SavedDir){ [IO.Directory]::Delete($SavedDir, $true) }
  Write-Host 'Мод удалён. Seamless Co-op остался.'
} catch {
  Write-Host ''
  Write-Host ('ОШИБКА: ' + $_.Exception.Message)
  exit 1
}
