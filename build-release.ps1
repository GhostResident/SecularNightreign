$ErrorActionPreference = 'Stop'
$cargo = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe'
if (-not (Test-Path $cargo)) { $cargo = 'cargo' }
& $cargo build --release --locked
New-Item -ItemType Directory -Force dist | Out-Null
Copy-Item target\release\secular-nightreign-launcher.exe dist\SecularNightreign.exe -Force
$hash = (Get-FileHash dist\SecularNightreign.exe -Algorithm SHA256).Hash.ToLowerInvariant()
"$hash  SecularNightreign.exe" | Set-Content dist\SecularNightreign.exe.sha256 -Encoding ascii
Write-Host "Built: dist\SecularNightreign.exe"
