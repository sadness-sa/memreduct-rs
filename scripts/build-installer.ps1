param([switch]$VerifyInstall)
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
$hash = (Get-FileHash $setup -Algorithm SHA256).Hash.ToLowerInvariant()
[IO.File]::WriteAllText((Join-Path $pwd 'dist/SHA256SUMS.txt'), "$hash  $([IO.Path]::GetFileName($setup))`n", (New-Object Text.UTF8Encoding $false))

if (-not $VerifyInstall) { return }
if (-not $env:RUNNER_TEMP) { throw 'Verification requires RUNNER_TEMP in an isolated Windows runner' }

$install = Join-Path $env:RUNNER_TEMP 'MemReduct RS Install Test'
$p = Start-Process -FilePath $setup -ArgumentList @('/S', "/D=$install") -Wait -PassThru
if ($p.ExitCode -ne 0) { throw "Install failed: $($p.ExitCode)" }
foreach ($file in @('memreduct-rs.exe', 'memreduct-cli.exe', 'Uninstall.exe', 'LICENSE', 'README.md')) {
    if (-not (Test-Path (Join-Path $install $file))) { throw "Missing installed file $file" }
}
$status = & (Join-Path $install 'memreduct-cli.exe') status --json | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $status.physical_total -le 0) { throw 'Installed CLI smoke test failed' }
# A visible update must start installing into the remembered custom directory without navigation.
$marker = Join-Path $install 'user-note.txt'
[IO.File]::WriteAllText($marker, 'preserve user files')
Remove-Item (Join-Path $install 'README.md')
if (-not ('InstallerUi' -as [type])) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstallerUi {
  [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
}
'@
}
$running = Start-Process -FilePath (Join-Path $install 'memreduct-rs.exe') -PassThru
Start-Sleep -Seconds 2
$update = Start-Process -FilePath $setup -PassThru
try {
    $deadline = [DateTime]::UtcNow.AddSeconds(30)
    while (-not (Test-Path (Join-Path $install 'README.md')) -and [DateTime]::UtcNow -lt $deadline) {
        if ($update.HasExited) { throw 'Update exited before restoring files' }
        Start-Sleep -Milliseconds 200
    }
    if (-not (Test-Path (Join-Path $install 'README.md'))) { throw 'Update required wizard navigation instead of installing directly' }
    if (-not $running.WaitForExit(5000)) { throw 'Installer did not gracefully close the existing application' }
    $savedPath = (Get-ItemProperty 'HKLM:\Software\MemReductRs').InstallDir
    if ($savedPath -ne $install -or (Get-Content $marker -Raw) -ne 'preserve user files') { throw 'Update did not preserve path or user files' }
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not $update.HasExited -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 300
        $update.Refresh()
        if ($update.MainWindowHandle -ne 0) {
            [InstallerUi]::SendMessage($update.MainWindowHandle, 0x0111, [IntPtr]1, [IntPtr]::Zero) | Out-Null
        }
    }
    if (-not $update.WaitForExit(3000) -or $update.ExitCode -ne 0) { throw 'Visible update did not finish successfully' }
} finally {
    if (-not $running.HasExited) { Stop-Process -Id $running.Id -Force }
    if (-not $update.HasExited) { Stop-Process -Id $update.Id -Force }
    $running.Dispose(); $update.Dispose()
}
$uninstaller = Join-Path $install 'Uninstall.exe'
$p = Start-Process -FilePath $uninstaller -ArgumentList @('/S', "_?=$install") -Wait -PassThru
if ($p.ExitCode -ne 0 -or (Test-Path (Join-Path $install 'memreduct-rs.exe'))) { throw 'Uninstall smoke test failed' }
