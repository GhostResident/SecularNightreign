# Установка мода «Секулярный Nightreign» из этого пакета (папки с version.txt, mod, scripts). Совместим с Windows PowerShell 5.1.
#   install.ps1                  - спросит папку игры, если не найдёт сам
#   install.ps1 -Game <папка>    - без вопросов о папке
#   -Auto                        - запуск из ярлыка: без вопросов, ярлык не пересоздаётся
param([string]$Game, [switch]$Auto)
$ErrorActionPreference = 'Stop'
$pkg   = Split-Path $PSScriptRoot
$files = Join-Path $pkg 'mod'
. (Join-Path $PSScriptRoot 'common.ps1')
$version = (Get-Content (Join-Path $pkg 'version.txt') -Encoding UTF8 | Select-Object -First 1).Trim()

if(-not $Game){ $Game = Find-Game -Auto:$Auto }
if(-not (Test-GameDir $Game)){ throw 'Папка с игрой не найдена. Установка отменена.' }
$game = $Game
if(-not $Auto){ Write-Host "Мод версии $version. Папка игры: $game" }
if(Test-GameRunning){ throw 'Закройте игру перед установкой.' }

$backup = Join-Path $game 'secular_backup'
New-Item -ItemType Directory -Force $backup | Out-Null

# ---- 1. механики: regulation.bin ----
$reg      = Join-Path $game 'regulation.bin'
$regBk    = Join-Path $backup 'regulation.bin'
$regMark  = Join-Path $backup 'installed_regulation.txt'   # sha256 regulation.bin, поставленного модом
$modSha   = Get-Sha (Join-Path $files 'regulation.bin')
$cur      = Get-Sha $reg
$bkSha    = if(Test-Path $regBk){ Get-Sha $regBk } else { $null }
$ourSha   = if(Test-Path $regMark){ (Get-Content $regMark | Select-Object -First 1).Trim() } else { $null }
if($cur -ne $modSha){
  $original = ($cur -eq $VanillaRegulationSha) -or ($bkSha -and $cur -eq $bkSha)
  # прошлая версия мода: записана при установке; у старых пакетов отметки нет — тогда признак мода
  # (оригинал в резервной копии, тексты мода в папке игры) при той же версии игры
  $previous = ($ourSha -and $cur -eq $ourSha) -or
              (-not $ourSha -and $bkSha -eq $VanillaRegulationSha -and (Test-Path (Join-Path $game 'secular_text.bin')) -and
               (Get-Item (Join-Path $game 'nightreign.exe')).VersionInfo.FileVersion -eq $GameExeVersion)
  if(-not $original -and -not $previous){
    $ver = (Get-Item (Join-Path $game 'nightreign.exe')).VersionInfo.FileVersion
    $msg = "regulation.bin (механики) в игре отличается от версии 1.03.3, под которую собран мод (версия игры: $ver). Если игра обновилась, мод нужно обновить под неё."
    if($Auto){ throw $msg }
    Write-Host ''
    Write-Host "ВНИМАНИЕ: $msg"
    $a = Read-Host 'Всё равно установить? (да/нет)'
    if($a -notmatch '^(д|да|y|yes)$'){ throw 'Установка отменена.' }
    if(-not $bkSha){ Copy-Item $reg $regBk }
  }
  elseif($original -and -not $bkSha){ Copy-Item $reg $regBk }
  Copy-Item (Join-Path $files 'regulation.bin') $reg -Force
  Write-Host 'механики: regulation.bin установлен'
}
Set-Content $regMark $modSha

# ---- 2. библиотека мода и её данные ----
$dll = Join-Path $game 'dinput8.dll'
$modDllSha = Get-Sha (Join-Path $files 'dinput8.dll')
if((Test-Path $dll) -and -not (Test-Path (Join-Path $backup 'dinput8.dll')) -and (Get-Sha $dll) -ne $modDllSha -and -not (Test-Path (Join-Path $game 'secular_text.bin'))){
  Copy-Item $dll (Join-Path $backup 'dinput8.dll')
  Write-Host 'чужой dinput8.dll сохранён в secular_backup (он будет заменён)'
}
foreach($n in 'dinput8.dll','secular_text.bin','secular_mute.txt'){
  $to = Join-Path $game $n; $from = Join-Path $files $n
  if(-not (Test-Path $to) -or (Get-Sha $to) -ne (Get-Sha $from)){ Copy-Item $from $to -Force; Write-Host "установлен $n" }
}

# ---- 3. язык: тексты мода русские ----
$langSet = $false
foreach($n in 'steam_emu.ini','OnlineFix.ini'){
  $ini = Join-Path $game $n
  if(-not (Test-Path $ini)){ continue }
  $t = [IO.File]::ReadAllText($ini)
  $t2 = [regex]::Replace($t, '(?m)^#?[ \t]*Language=[^\r\n]*', 'Language=russian')
  if($t2 -ne $t){
    if(-not (Test-Path (Join-Path $backup $n))){ Copy-Item $ini (Join-Path $backup $n) }
    [IO.File]::WriteAllText($ini, $t2)
    Write-Host "язык игры: русский ($n)"
  }
  $langSet = $true
}
if(-not $langSet -and -not $Auto){ Write-Host 'Язык игры должен быть русским (Steam: Свойства игры -> Язык).' }

# ---- 4. копия мода в папке игры (из неё работает ярлык) и ярлык ----
$home_ = Join-Path $game 'SecularMod'
if([IO.Path]::GetFullPath($pkg).TrimEnd('\') -ne [IO.Path]::GetFullPath($home_).TrimEnd('\')){
  if(Test-Path $home_){
    # лог ярлыка сохраняем
    foreach($d in Get-ChildItem $home_ | Where-Object { $_.Name -ne 'launch.log' }){
      if($d.PSIsContainer){ [IO.Directory]::Delete($d.FullName, $true) } else { [IO.File]::Delete($d.FullName) }
    }
  }
  New-Item -ItemType Directory -Force $home_ | Out-Null
  foreach($n in 'version.txt','mod','scripts','ПРОЧТИ.txt','Удалить мод.bat'){
    $p = Join-Path $pkg $n
    if(Test-Path $p){ Copy-Item $p $home_ -Recurse -Force }
  }
}
Save-GamePath $game
if(-not $Auto){
  New-DesktopShortcut $game (Join-Path $home_ 'scripts\launch.ps1')
  . (Join-Path $PSScriptRoot 'config.ps1')
  Write-Host "ярлык на рабочем столе: «$ShortcutName»"
  Write-Host ''
  Write-Host 'Готово. Запускайте игру ярлыком на рабочем столе: он сам обновляет мод и запускает игру'
  Write-Host '(через Seamless Co-op, если он установлен).'
  Write-Host 'Признак работы мода: «НАЖМИТЕ ЛЮБУЮ КНОПКУ (МОД АКТИВЕН, ...)» на титульном экране.'
}
