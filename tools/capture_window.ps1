# Startet ctxmenu, fotografiert das Fenster und beendet es wieder.
#
# Dient der Sichtpruefung ohne Fernwartung: eine gerenderte Oberflaeche laesst
# sich sonst nur behaupten, nicht belegen.
#
# Aufruf:  pwsh -File tools\capture_window.ps1 -Out shot.png [-Args '--synthetic','2000']

param(
    [string]$Exe = 'target\x86_64-pc-windows-msvc\release\ctxmenu.exe',
    [string]$Out = 'window.png',
    [string[]]$AppArgs = @(),
    [int]$WaitSeconds = 6
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class Win {
    [StructLayout(LayoutKind.Sequential)]
    public struct RECT { public int Left, Top, Right, Bottom; }

    [DllImport("user32.dll")]
    public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);

    [DllImport("user32.dll")]
    public static extern bool SetForegroundWindow(IntPtr hWnd);

    [DllImport("user32.dll")]
    public static extern bool IsWindowVisible(IntPtr hWnd);
}
'@

$exePath = (Resolve-Path $Exe).Path
$proc = Start-Process -FilePath $exePath -ArgumentList $AppArgs -PassThru
Start-Sleep -Seconds $WaitSeconds

try {
    $proc.Refresh()
    $hwnd = $proc.MainWindowHandle
    if ($hwnd -eq [IntPtr]::Zero) { throw "Kein Fenster gefunden (MainWindowHandle ist 0)" }

    [void][Win]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 800

    $rect = New-Object Win+RECT
    if (-not [Win]::GetWindowRect($hwnd, [ref]$rect)) { throw "GetWindowRect fehlgeschlagen" }

    $w = $rect.Right - $rect.Left
    $h = $rect.Bottom - $rect.Top
    if ($w -le 0 -or $h -le 0) { throw "Unsinnige Fenstergroesse ${w}x${h}" }

    # Bildschirmkopie statt PrintWindow: OpenGL-Inhalt taucht bei PrintWindow
    # je nach Treiber als schwarze Flaeche auf.
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    $g.Dispose()

    $bmp.Save([System.IO.Path]::GetFullPath($Out), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()

    Write-Output "geschrieben: $Out (${w}x${h})"
} finally {
    if (-not $proc.HasExited) { $proc | Stop-Process -Force }
}
