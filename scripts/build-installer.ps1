$ErrorActionPreference = 'Stop'
$compiler = Join-Path ${env:ProgramFiles(x86)} 'NSIS\makensis.exe'
if (-not (Test-Path $compiler)) {
    choco install nsis --no-progress -y
    if ($LASTEXITCODE -ne 0) { throw 'NSIS installation failed' }
}
$versionLine = Select-String -Path Cargo.toml -Pattern '^version = "([0-9.]+)"' | Select-Object -First 1
$version = $versionLine.Matches[0].Groups[1].Value
if (-not $version) { throw 'Missing package version' }
New-Item dist -ItemType Directory -Force | Out-Null
& $compiler "/DVERSION=$version" installer/memreduct-rs.nsi
if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
$setup = (Resolve-Path "dist/MemReduct-RS-$version-x64-Setup.exe").Path
$install = Join-Path $env:RUNNER_TEMP 'MemReductRs-InstallTest'
$p = Start-Process -FilePath $setup -ArgumentList @('/S', "/D=$install") -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "Install failed: $($p.ExitCode)" }
foreach ($file in @('memreduct-rs.exe', 'memreduct-cli.exe', 'Uninstall.exe', 'LICENSE', 'README.md')) {
    if (-not (Test-Path (Join-Path $install $file))) { throw "Missing installed file $file" }
}
$status = & (Join-Path $install 'memreduct-cli.exe') status --json | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $status.physical_total -le 0) { throw 'Installed CLI smoke test failed' }
$uninstaller = Join-Path $install 'Uninstall.exe'
$p = Start-Process -FilePath $uninstaller -ArgumentList @('/S', "_?=$install") -Wait -PassThru
if ($p.ExitCode -ne 0 -or (Test-Path (Join-Path $install 'memreduct-rs.exe'))) { throw 'Uninstall smoke test failed' }
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $pwd 'dist/SHA256SUMS.txt'), "$hash  $([IO.Path]::GetFileName($setup))`n", (New-Object Text.UTF8Encoding $false))
