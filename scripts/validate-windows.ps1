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
$originalLocalAppData = $env:LOCALAPPDATA
$env:LOCALAPPDATA = Join-Path $env:RUNNER_TEMP 'MemReductRs-UiTest'
$process = Start-Process -FilePath $gui -PassThru
try {
    Start-Sleep -Seconds 3
    $process.Refresh()
    if ($process.HasExited -or $process.MainWindowHandle -eq 0) { throw 'GUI did not create a window' }
    Add-Type -AssemblyName System.Drawing
    Add-Type @'
using System;
using System.Runtime.InteropServices;
using System.Text;
public static class NativeUi {
  [StructLayout(LayoutKind.Sequential)] public struct Rect { public int Left, Top, Right, Bottom; }
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hwnd, out Rect rect);
  [DllImport("user32.dll", EntryPoint="SendMessageW")] public static extern IntPtr SendMessage(IntPtr hwnd, uint msg, IntPtr wp, IntPtr lp);
  [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr dc, uint flags);
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hwnd);
  [DllImport("user32.dll")] public static extern IntPtr GetDlgItem(IntPtr hwnd, int id);
  [DllImport("user32.dll", EntryPoint="GetWindowLongPtrW")] public static extern IntPtr GetWindowLong(IntPtr hwnd, int index);
  [DllImport("user32.dll", EntryPoint="SendMessageW", ExactSpelling=true, CharSet=CharSet.Unicode, SetLastError=true)] public static extern IntPtr SendText(IntPtr hwnd, uint msg, IntPtr wp, string text);
  [DllImport("user32.dll", EntryPoint="SendMessageW", ExactSpelling=true, CharSet=CharSet.Unicode)] public static extern IntPtr ReadText(IntPtr hwnd, uint msg, IntPtr capacity, StringBuilder text);
}
'@
    $window = $process.MainWindowHandle
    $style = [NativeUi]::GetWindowLong($window, -16).ToInt64()
    if (($style -band 0x00c00000) -ne 0) { throw 'Main surface still has a native caption or border' }
    [NativeUi]::SetForegroundWindow($window) | Out-Null
    $rect = New-Object NativeUi+Rect
    if (-not [NativeUi]::GetWindowRect($window, [ref]$rect)) { throw 'No window rectangle' }
    $scale = ($rect.Right - $rect.Left) / 820.0
    $x = $rect.Left + [int](40 * $scale)
    $y = $rect.Top + [int](20 * $scale)
    $point = (($y -band 0xffff) -shl 16) -bor ($x -band 0xffff)
    if ([NativeUi]::SendMessage($window, 0x0084, [IntPtr]::Zero, [IntPtr]$point).ToInt64() -ne 2) { throw 'Custom header must support native dragging' }
    function Capture-Window([string]$name) {
        Start-Sleep -Milliseconds 500
        $captureRect = New-Object NativeUi+Rect
        [NativeUi]::GetWindowRect($window, [ref]$captureRect) | Out-Null
        $bitmap = New-Object System.Drawing.Bitmap ($captureRect.Right - $captureRect.Left), ($captureRect.Bottom - $captureRect.Top)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        try {
            $dc = $graphics.GetHdc()
            try { if (-not [NativeUi]::PrintWindow($window, $dc, 2)) { throw 'Window capture failed' } }
            finally { $graphics.ReleaseHdc($dc) }
            $bitmap.Save((Join-Path $pwd "dist/$name"))
        } finally { $graphics.Dispose(); $bitmap.Dispose() }
    }
    # Select smart without executing cleanup; existing configurations keep their preferences until selected.
    [NativeUi]::SendMessage($window, 0x0111, [IntPtr]120, [IntPtr]::Zero) | Out-Null
    Capture-Window 'ui-preview.png'
    [NativeUi]::SendMessage($window, 0x0111, [IntPtr]104, [IntPtr]::Zero) | Out-Null
    $threshold = [NativeUi]::GetDlgItem($window, 200)
    if ($threshold -eq [IntPtr]::Zero) { throw 'Settings must open inside the main window' }
    Capture-Window 'settings-preview.png'
    [NativeUi]::SendText($threshold, 0x000c, [IntPtr]::Zero, '100') | Out-Null
    [NativeUi]::SendMessage($window, 0x0111, [IntPtr]310, [IntPtr]::Zero) | Out-Null
    $text = New-Object Text.StringBuilder 256
    [NativeUi]::ReadText([NativeUi]::GetDlgItem($window, 220), 0x000d, [IntPtr]$text.Capacity, $text) | Out-Null
    if ($text.ToString() -notmatch '50..99') { throw "Settings did not reject invalid threshold: $($text.ToString())" }
    [NativeUi]::SendText($threshold, 0x000c, [IntPtr]::Zero, '90') | Out-Null
    [NativeUi]::SendMessage($window, 0x0111, [IntPtr]310, [IntPtr]::Zero) | Out-Null
    if ([NativeUi]::GetDlgItem($window, 200) -ne [IntPtr]::Zero) { throw 'Saving settings must restore overview' }
    $config = Get-Content (Join-Path $env:LOCALAPPDATA 'MemReductRs/config.toml') -Raw
    if ($config -notmatch 'threshold_percent = 90' -or $config -notmatch 'auto_profile = "smart"') { throw 'Saved settings are incorrect' }
    [NativeUi]::SendMessage($window, 0x0111, [IntPtr]109, [IntPtr]::Zero) | Out-Null
    [NativeUi]::SendMessage($window, 0x0111, [IntPtr]108, [IntPtr]::Zero) | Out-Null
    [NativeUi]::SendMessage($window, 0x8004, [IntPtr]::Zero, [IntPtr]::Zero) | Out-Null
    if (-not $process.WaitForExit(5000)) { throw 'GUI did not shut down cleanly' }
    if ($process.ExitCode -ne 0) { throw 'GUI returned failure' }
} finally {
    if (-not $process.HasExited) { Stop-Process -Id $process.Id -Force }
    $process.Dispose()
    $env:LOCALAPPDATA = $originalLocalAppData
}
cargo run --release --example tray-preview --locked
if ($LASTEXITCODE -ne 0) { throw 'Tray rendering preview failed' }
$preview = [Drawing.Bitmap]::FromFile((Join-Path $pwd 'dist/tray-preview.bmp'))
try { $preview.Save((Join-Path $pwd 'dist/tray-preview.png'), [Drawing.Imaging.ImageFormat]::Png) } finally { $preview.Dispose() }
