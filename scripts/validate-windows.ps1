$ErrorActionPreference = 'Stop'
New-Item dist -ItemType Directory -Force | Out-Null
$memory = & .\target\release\memreduct-cli.exe status --json | ConvertFrom-Json
if ($LASTEXITCODE -ne 0 -or $memory.physical_total -le 0 -or $memory.physical_available -gt $memory.physical_total) { throw 'Invalid memory snapshot' }
$gui = (Resolve-Path '.\target\release\memreduct-rs.exe').Path
$cli = (Resolve-Path '.\target\release\memreduct-cli.exe').Path
$mt = (Get-Command mt.exe -ErrorAction Stop).Source
& $mt "-inputresource:$gui;#1" '-out:dist/gui.manifest'
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract GUI manifest' }
& $mt "-inputresource:$cli;#1" '-out:dist/cli.manifest'
if ($LASTEXITCODE -ne 0) { throw 'Cannot extract CLI manifest' }
if ((Get-Content dist/gui.manifest -Raw) -notmatch 'level="requireAdministrator"') { throw 'GUI must request elevation' }
if ((Get-Content dist/cli.manifest -Raw) -notmatch 'level="asInvoker"') { throw 'CLI status must work without elevation' }
$process = Start-Process -FilePath $gui -PassThru
try {
    Start-Sleep -Seconds 3
    $process.Refresh()
    if ($process.HasExited -or $process.MainWindowHandle -eq 0) { throw 'GUI did not create a window' }
    Add-Type -AssemblyName System.Drawing
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class NativeUi {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
  [DllImport("user32.dll")] public static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
}
'@
    [NativeUi]::SetForegroundWindow($process.MainWindowHandle) | Out-Null
    Start-Sleep -Milliseconds 500
    $rect = New-Object NativeUi+Rect
    if (-not [NativeUi]::GetWindowRect($process.MainWindowHandle, [ref]$rect)) { throw 'No window rectangle' }
    $bitmap = New-Object System.Drawing.Bitmap ($rect.Right - $rect.Left), ($rect.Bottom - $rect.Top)
    $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
    try {
        $dc = $graphics.GetHdc()
        try { if (-not [NativeUi]::PrintWindow($process.MainWindowHandle, $dc, 2)) { throw 'Window capture failed' } }
        finally { $graphics.ReleaseHdc($dc) }
        $bitmap.Save((Join-Path $pwd 'dist/ui-preview.png'))
    } finally { $graphics.Dispose(); $bitmap.Dispose() }
    [NativeUi]::SendMessage($process.MainWindowHandle, 0x8004, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $process.WaitForExit(5000)) { throw 'GUI did not shut down cleanly' }
    if ($process.ExitCode -ne 0) { throw 'GUI returned failure' }
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    $process.Dispose()
}
