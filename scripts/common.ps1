# Общие функции установщика: поиск папки игры, загрузка мода с GitHub, ярлык, Seamless Co-op,
# обновление игры цепочкой патчей до 1.03.3.
$VanillaRegulationSha = '876A3CA279A4561D0C69F81FE5E510C75C59AB9A8A201694F8CF4A42C91E0268'   # ELDEN RING NIGHTREIGN 1.03.3
$GameExeVersion = '1.3.3.0'
$SavedDir = Join-Path $env:LOCALAPPDATA 'SecularNightreign'

# версии nightreign.exe, между которыми существуют патчи (как в nightreign-updater-tool)
$GameChain = @('1.3.0', '1.3.1', '1.3.2', '1.3.3')

function Test-GameDir($d){ $d -and (Test-Path (Join-Path $d 'nightreign.exe')) }

function Find-GameNear($start){
  # папка игры, внутри которой (или рядом с которой) лежит $start
  $d = $start
  while($d){
    if(Test-GameDir $d){ return $d }
    if(Test-GameDir (Join-Path $d 'Game')){ return (Join-Path $d 'Game') }
    $parent = Split-Path $d
    if(-not $parent -or $parent -eq $d){ break }
    $d = $parent
  }
  return $null
}

function Find-Game([switch]$Auto) {
  # 1) пакет лежит внутри папки игры (или рядом с ней)
  $g = Find-GameNear (Split-Path $PSScriptRoot)
  if($g -or $Auto){ return $g }
  # 2) мод уже стоит: папка игры записана при прошлой установке
  $saved = Join-Path $SavedDir 'game.txt'
  if(Test-Path $saved){
    $g = (Get-Content $saved -Encoding UTF8 | Select-Object -First 1)
    if(Test-GameDir $g){
      $a = Read-Host "Игра с модом: $g`nИспользовать эту папку? (да/нет)"
      if($a -match '^(д|да|y|yes)$'){ return $g }
    }
  }
  # 3) библиотеки Steam
  $steam = $null
  try { $steam = (Get-ItemProperty 'HKCU:\Software\Valve\Steam' -ErrorAction Stop).SteamPath } catch {}
  if($steam){
    $libs = @($steam)
    $vdf = Join-Path $steam 'steamapps\libraryfolders.vdf'
    if(Test-Path $vdf){ foreach($m in [regex]::Matches((Get-Content $vdf -Raw), '"path"\s+"([^"]+)"')){ $libs += $m.Groups[1].Value -replace '\\\\','\' } }
    foreach($l in $libs | Select-Object -Unique){
      $g = Join-Path $l 'steamapps\common\ELDEN RING NIGHTREIGN\Game'
      if(Test-GameDir $g){
        $a = Read-Host "Найдена игра: $g`nУстановить сюда? (да/нет)"
        if($a -match '^(д|да|y|yes)$'){ return $g }
      }
    }
  }
  # 4) выбрать вручную
  Add-Type -AssemblyName System.Windows.Forms
  $dlg = New-Object System.Windows.Forms.FolderBrowserDialog
  $dlg.Description = 'Выберите папку игры ELDEN RING NIGHTREIGN (где лежит nightreign.exe или папка Game)'
  while($dlg.ShowDialog() -eq 'OK'){
    $p = $dlg.SelectedPath
    if(Test-GameDir $p){ return $p }
    if(Test-GameDir (Join-Path $p 'Game')){ return (Join-Path $p 'Game') }
    [System.Windows.Forms.MessageBox]::Show('В этой папке нет nightreign.exe. Выберите папку игры или папку Game внутри неё.') | Out-Null
  }
  return $null
}

function Save-GamePath($game){
  New-Item -ItemType Directory -Force $SavedDir | Out-Null
  Set-Content (Join-Path $SavedDir 'game.txt') $game -Encoding UTF8
}

function Get-Sha($path){
  # без Get-FileHash: при запуске из другой оболочки PowerShell 5.1 может подхватить чужой модуль, где его нет
  $fs = [IO.File]::OpenRead($path); $sha = [Security.Cryptography.SHA256]::Create()
  try { return ([BitConverter]::ToString($sha.ComputeHash($fs)) -replace '-','') } finally { $sha.Dispose(); $fs.Dispose() }
}

function Test-GameRunning { [bool](Get-Process nightreign -ErrorAction SilentlyContinue) }

# ---- загрузка мода с GitHub ----
function Get-ModPackage {
  # скачивает текущую версию мода и возвращает папку распакованного пакета (в ней version.txt, mod, scripts)
  . (Join-Path $PSScriptRoot 'config.ps1')
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('SecularNightreign_' + [guid]::NewGuid().ToString('N'))
  New-Item -ItemType Directory -Force $tmp | Out-Null
  $zip = Join-Path $tmp 'mod.zip'
  $ProgressPreference = 'SilentlyContinue'
  Invoke-WebRequest -Uri $ZipUrl -OutFile $zip -UseBasicParsing -TimeoutSec 120
  Expand-Archive -Path $zip -DestinationPath (Join-Path $tmp 'x') -Force
  $ver = Get-ChildItem (Join-Path $tmp 'x') -Recurse -Filter 'version.txt' | Select-Object -First 1
  if(-not $ver){ throw 'в скачанном архиве нет мода (version.txt)' }
  return $ver.DirectoryName
}

function Remove-ModPackage($pkg){
  # удаляет временную папку, созданную Get-ModPackage
  $tmp = Split-Path (Split-Path $pkg)
  if((Split-Path $tmp -Leaf) -like 'SecularNightreign_*'){ [IO.Directory]::Delete($tmp, $true) }
}

function Get-RemoteVersion {
  . (Join-Path $PSScriptRoot 'config.ps1')
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  $r = Invoke-WebRequest -Uri "$RawUrl/version.txt" -UseBasicParsing -TimeoutSec 8
  $s = if($r.Content -is [byte[]]){ [Text.Encoding]::UTF8.GetString($r.Content) } else { [string]$r.Content }
  return [version]($s.Trim([char]0xFEFF, " ", "`r", "`n", "`t"))
}

# ---- ярлык ----
function New-DesktopShortcut($game, $launcher){
  . (Join-Path $PSScriptRoot 'config.ps1')
  $desk = [Environment]::GetFolderPath('Desktop')
  $sh = New-Object -ComObject WScript.Shell
  $lnk = $sh.CreateShortcut((Join-Path $desk ($ShortcutName + '.lnk')))
  $lnk.TargetPath = Join-Path $env:SystemRoot 'System32\WindowsPowerShell\v1.0\powershell.exe'
  $lnk.Arguments = "-NoProfile -WindowStyle Hidden -ExecutionPolicy Bypass -File `"$launcher`""
  $lnk.WorkingDirectory = $game
  $lnk.IconLocation = (Join-Path $game 'nightreign.exe') + ',0'
  $lnk.Description = 'Запуск Elden Ring Nightreign с секулярным модом (проверяет обновления мода)'
  $lnk.WindowStyle = 7
  $lnk.Save()
}

function Remove-DesktopShortcut {
  . (Join-Path $PSScriptRoot 'config.ps1')
  $p = Join-Path ([Environment]::GetFolderPath('Desktop')) ($ShortcutName + '.lnk')
  if(Test-Path $p){ [IO.File]::Delete($p) }
}

# ---- Seamless Co-op ----
function Install-Seamless($zip, $game){
  # распаковывает архив Seamless Co-op в папку игры; уже существующие .ini не трогает
  $tmp = Join-Path ([IO.Path]::GetTempPath()) ('SecularNightreign_sc_' + [guid]::NewGuid().ToString('N'))
  Expand-Archive -Path $zip -DestinationPath $tmp -Force
  try {
    $exe = Get-ChildItem $tmp -Recurse -Filter 'nrsc_launcher.exe' | Select-Object -First 1
    if(-not $exe){ Write-Host 'Seamless Co-op: в архиве нет nrsc_launcher.exe, пропущено'; return }
    $root = $exe.DirectoryName; $n = 0
    foreach($f in Get-ChildItem $root -Recurse -File){
      $rel = $f.FullName.Substring($root.Length).TrimStart('\')
      $to = Join-Path $game $rel
      if($f.Extension -eq '.ini' -and (Test-Path $to)){ continue }
      New-Item -ItemType Directory -Force (Split-Path $to) | Out-Null
      Copy-Item $f.FullName $to -Force; $n++
    }
    Write-Host "Seamless Co-op: установлен ($n файлов)"
  } finally { [IO.Directory]::Delete($tmp, $true) }
}

# ---- обновление игры до 1.03.3 ----
# Логика из nightreign-updater-tool: версия из nightreign.exe, цепочка патчей
# 1.3.0→1.3.1→1.3.2 (ElAmigos с GitHub: exe-установщик + elamigos-1.bin из двух
# частей, тихая установка Inno Setup) и 1.3.2→1.3.3 (архив поверх Game\).

function Get-GameVersion($game){
  try { $raw = (Get-Item (Join-Path $game 'nightreign.exe')).VersionInfo.FileVersion } catch { return $null }
  if(-not $raw){ return $null }
  $ints = @()
  foreach($p in ($raw.Trim() -replace '\s', '').Split('.')){
    $n = 0
    if([int]::TryParse($p, [ref]$n)){ $ints += $n }
  }
  if($ints.Count -eq 0){ return $null }
  while($ints.Count -gt 3 -and $ints[$ints.Count - 1] -eq 0){ $ints = $ints[0..($ints.Count - 2)] }
  while($ints.Count -lt 3){ $ints += 0 }
  return (($ints | Select-Object -First 3) -join '.')
}

function Get-ChainIndex($ver){
  if(-not $ver){ return -1 }
  for($i = 0; $i -lt $GameChain.Count; $i++){ if($GameChain[$i] -eq $ver){ return $i } }
  for($i = 0; $i -lt $GameChain.Count; $i++){
    if($GameChain[$i].StartsWith($ver + '.') -or $ver.StartsWith($GameChain[$i] + '.')){ return $i }
  }
  return -1
}

function Invoke-PatchDownload($url, $dest){
  # скачивание с докачкой и повторами; прогресс в консоли
  [Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12
  for($attempt = 1; $attempt -le 5; $attempt++){
    try {
      $existing = 0
      if(Test-Path $dest){ $existing = (Get-Item $dest).Length }
      if($existing -gt 0){ Write-Host ('  продолжаю с ' + [math]::Round($existing / 1MB) + ' МБ') }
      $req = [Net.HttpWebRequest]::Create($url)
      $req.UserAgent = 'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/124.0 Safari/537.36'
      $req.AllowAutoRedirect = $true
      if($existing -gt 0){ $req.AddRange($existing) }
      $resp = $req.GetResponse()
      $resume = ($existing -gt 0) -and ($resp.StatusCode -eq [Net.HttpStatusCode]::PartialContent)
      $total = [long]$resp.ContentLength
      if($resume){ $total += $existing }
      $src = $resp.GetResponseStream()
      $mode = if($resume){ [IO.FileMode]::Append } else { [IO.FileMode]::Create }
      $dst = [IO.File]::Open($dest, $mode, [IO.FileAccess]::Write)
      try {
        $buf = New-Object byte[] 524288
        $got = if($resume){ $existing } else { 0 }
        $last = Get-Date
        while(($n = $src.Read($buf, 0, $buf.Length)) -gt 0){
          $dst.Write($buf, 0, $n)
          $got += $n
          if(((Get-Date) - $last).TotalMilliseconds -gt 500){
            $last = Get-Date
            $pct = if($total -gt 0){ [int]([double]$got * 100 / $total) } else { 0 }
            Write-Progress -Activity ('Скачивание ' + (Split-Path $dest -Leaf)) -Status ('{0:N0} из {1:N0} МБ' -f ($got / 1MB), ($total / 1MB)) -PercentComplete $pct
          }
        }
      } finally {
        $dst.Dispose(); $src.Dispose(); $resp.Close()
        Write-Progress -Activity ('Скачивание ' + (Split-Path $dest -Leaf)) -Completed
      }
      Write-Host ('  скачано: ' + (Split-Path $dest -Leaf) + ' (' + [math]::Round((Get-Item $dest).Length / 1MB) + ' МБ)')
      return $true
    } catch {
      $code = $null
      try { $code = [int]$_.Exception.GetBaseException().Response.StatusCode } catch {}
      Write-Host ('  ошибка загрузки: ' + $_.Exception.Message)
      if($code -eq 404 -or $code -eq 403){ return $false }
      if($attempt -lt 5){ Write-Host ('  повтор через ' + (2 * $attempt) + ' с'); Start-Sleep -Seconds (2 * $attempt) }
    }
  }
  return $false
}

function Join-PatchParts($parts, $dest){
  # склейка частей elamigos-1.bin: это сырые куски файла, не архив
  $tmp = $dest + '.tmp'
  $dst = [IO.File]::Create($tmp)
  try {
    foreach($p in $parts){
      $src = [IO.File]::OpenRead($p)
      try { $src.CopyTo($dst, 1MB) } finally { $src.Dispose() }
    }
  } finally { $dst.Dispose() }
  Move-Item $tmp $dest -Force
}

$RapidCrcSource = @'
using System;
using System.Text;
using System.Text.RegularExpressions;
using System.Runtime.InteropServices;
public class RapidCrcWin {
  public delegate bool EnumProc(IntPtr hWnd, IntPtr lParam);
  [DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr h, EnumProc f, IntPtr l);
  [DllImport("user32.dll")] public static extern int GetWindowText(IntPtr h, StringBuilder t, int c);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
  [DllImport("user32.dll")] public static extern bool PostMessage(IntPtr h, uint m, IntPtr w, IntPtr l);

  static IntPtr Find(){
    IntPtr found = IntPtr.Zero;
    EnumWindows(delegate(IntPtr h, IntPtr l){
      if(!IsWindowVisible(h)) return true;
      StringBuilder sb = new StringBuilder(256);
      GetWindowText(h, sb, 256);
      if(sb.ToString().IndexOf("RapidCRC", StringComparison.OrdinalIgnoreCase) >= 0){ found = h; return false; }
      return true;
    }, IntPtr.Zero);
    return found;
  }
  static bool Done(IntPtr h){
    // готово, когда в окне "N/N done" (числа равны); не закрываем недоставленное
    StringBuilder all = new StringBuilder();
    EnumChildWindows(h, delegate(IntPtr c, IntPtr l){
      StringBuilder sb = new StringBuilder(512);
      GetWindowText(c, sb, 512);
      if(sb.Length > 0) all.Append(sb).Append(" | ");
      return true;
    }, IntPtr.Zero);
    Match m = Regex.Match(all.ToString(), @"(\d+)\s*/\s*(\d+)\s*done");
    return m.Success && m.Groups[1].Value == m.Groups[2].Value;
  }
  // закрывает окно RapidCRC, если проверка файлов закончилась; true — закрыто
  public static bool CloseWhenDone(){
    IntPtr h = Find();
    if(h == IntPtr.Zero) return false;
    if(!Done(h)) return false;
    PostMessage(h, 0x0010, IntPtr.Zero, IntPtr.Zero);
    return true;
  }
}
'@

function Invoke-SilentInstaller($exe, $dir){
  # тихая установка Inno Setup; попутно закрывает окно проверки RapidCRC,
  # когда оно закончит (само оно не закрывается)
  if(-not ('RapidCrcWin' -as [type])){ Add-Type -TypeDefinition $RapidCrcSource }
  Write-Host ('  тихая установка: ' + (Split-Path $exe -Leaf))
  $p = Start-Process -FilePath $exe -ArgumentList ('/VERYSILENT /SUPPRESSMSGBOXES /NOCANCEL /NORESTART /DIR="' + $dir + '"') -PassThru
  $start = Get-Date
  $closed = $false
  while(-not $p.HasExited){
    Start-Sleep -Seconds 3
    if(-not $closed){
      try { if([RapidCrcWin]::CloseWhenDone()){ $closed = $true; Write-Host '  проверка файлов (RapidCRC) завершена, окно закрыто.' } } catch {}
    }
    if(((Get-Date) - $start).TotalMinutes -ge 40){
      try { $p.Kill() } catch {}
      throw 'Установщик патча не завершился за 40 минут.'
    }
  }
  if($p.ExitCode -ne 0 -and $p.ExitCode -ne 3010){ throw ('Установщик патча завершился с кодом ' + $p.ExitCode + '.') }
}

function Install-PatchElAmigos($to, $installRoot, $dlDir){
  # патч ElAmigos ($to: '1.3.1' или '1.3.2'): exe + 2 части bin, склейка, тихая установка
  . (Join-Path $PSScriptRoot 'config.ps1')
  $u = $PatchUrls[$to]
  $exeName = if($to -eq '1.3.1'){ 'Elden Ring Nightreign update 1.03.0 - 1.03.1.exe' } else { 'Elden Ring Nightreign update 1.03.1 - 1.03.2.exe' }
  $exeDest = Join-Path $dlDir $exeName
  $binDest = Join-Path $dlDir 'elamigos-1.bin'

  if(Test-Path $binDest){
    Write-Host '  elamigos-1.bin уже скачан, используется.'
  } else {
    $parts = @()
    foreach($i in 1, 2){
      $dest = Join-Path $dlDir ('elamigos-1.bin.part' + $i)
      Write-Host ('  скачиваю ' + (Split-Path $dest -Leaf) + '...')
      if(-not (Invoke-PatchDownload $u[('Part' + $i)] $dest)){ throw ('Не удалось скачать: ' + $u[('Part' + $i)]) }
      $parts += $dest
    }
    Write-Host '  склеиваю elamigos-1.bin...'
    Join-PatchParts $parts $binDest
    foreach($p in $parts){ Remove-Item $p -Force -ErrorAction SilentlyContinue }
  }
  Write-Host ('  скачиваю ' + $exeName + '...')
  if(-not (Invoke-PatchDownload $u['Exe'] $exeDest)){ throw ('Не удалось скачать: ' + $u['Exe']) }

  Invoke-SilentInstaller $exeDest $installRoot
  Remove-Item $exeDest, $binDest -Force -ErrorAction SilentlyContinue
}

function Install-Patch133($game, $dlDir){
  # патч 1.03.3: архив поверх Game\; лежит в packages установщика, при отсутствии — ссылка из config.ps1
  . (Join-Path $PSScriptRoot 'config.ps1')
  $zip = Join-Path (Split-Path $PSScriptRoot) 'packages\nightreign-patch-1.03.3.zip'
  if(-not (Test-Path $zip)){
    if($PatchUrls['1.3.3']['Zip']){
      $zip = Join-Path $dlDir 'nightreign-patch-1.03.3.zip'
      Write-Host '  скачиваю патч 1.03.3 (~92 МБ)...'
      if(-not (Invoke-PatchDownload $PatchUrls['1.3.3']['Zip'] $zip)){ throw ('Не удалось скачать: ' + $PatchUrls['1.3.3']['Zip']) }
    } else {
      throw ('В установщике нет ' + $zip + '. Обновите игру до 1.03.3 вручную или используйте полный установщик.')
    }
  }
  Write-Host '  распаковываю патч 1.03.3 в папку игры...'
  Expand-Archive -Path $zip -DestinationPath $game -Force
}

function Update-Game($game){
  # приводит игру к 1.03.3 (мод собран под неё); бросает исключение, если обновить не удалось
  $ver = Get-GameVersion $game
  if(-not $ver){ throw 'Не удалось определить версию игры (nightreign.exe без сведений о версии). Обновите игру до 1.03.3 вручную.' }
  Write-Host ('Версия игры: ' + $ver)
  $latest = $GameChain.Count - 1
  $idx = Get-ChainIndex $ver
  if($idx -eq $latest){ Write-Host 'Игра уже последней версии (1.03.3), обновление не нужно.'; return }
  try { if([version]$ver -gt [version]$GameChain[$latest]){ Write-Host 'ВНИМАНИЕ: игра новее 1.03.3, под которую собран мод. Обновление пропущено.'; return } } catch {}
  if($idx -lt 0){ throw ('Версия игры ' + $ver + ' не подходит для автообновления: нужны версии 1.03.0–1.03.3. Обновите игру вручную.') }

  # патчи ElAmigos ставятся в папку, внутри которой лежит Game\
  $installRoot = if((Split-Path $game -Leaf) -ieq 'Game'){ Split-Path $game } else { $game }
  $dlDir = Join-Path $env:USERPROFILE 'Downloads\NightreignUpdates'
  New-Item -ItemType Directory -Force $dlDir | Out-Null

  for($i = $idx; $i -lt $latest; $i++){
    $from = $GameChain[$i]
    $to = $GameChain[$i + 1]
    Write-Host ''
    Write-Host ('Обновление игры: ' + $from + ' → ' + $to + ' (скачивается из интернета, это долго)')
    if($to -eq '1.3.3'){ Install-Patch133 $game $dlDir }
    else { Install-PatchElAmigos $to $installRoot $dlDir }
    $now = Get-GameVersion $game
    if((Get-ChainIndex $now) -ne ($i + 1)){ throw ('После патча версия игры — ' + $now + ', ожидалась ' + $to + '. Патч не применился.') }
    Write-Host ('Версия после патча: ' + $now)
  }
  Write-Host 'Игра обновлена до 1.03.3.'
}
