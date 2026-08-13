# Startet ctxmenu, fotografiert das Fenster und beendet es wieder.
#
# Dient der Sichtpruefung ohne Fernwartung: eine gerenderte Oberflaeche laesst
# sich sonst nur behaupten, nicht belegen.
#
# ACHTUNG, gelernt: der Rueckfallweg kopiert einen Bildschirmausschnitt. Liegt
# ein anderes Fenster darueber -- ein immer-oben-Terminal etwa -- landet dessen
# Inhalt in der Datei, und auf einem Nebenbildschirm ist das schnell privat.
# Deshalb: nur auf einem Bildschirm aufnehmen, von dem man weiss, was darauf
# liegt, und das Ergebnis vor dem Weitergeben ansehen.
#
# Aufruf:  pwsh -File tools\capture_window.ps1 -Out shot.png [-Args '--synthetic','2000']

param(
    [string]$Exe = 'target\x86_64-pc-windows-msvc\release\ctxmenu.exe',
    [string]$Out = 'window.png',
    [string[]]$AppArgs = @(),
    [int]$WaitSeconds = 6,
    # Auf welchen Bildschirm das Fenster geschoben wird, bevor es fotografiert
    # wird. Vorgabe links: der mittlere Bildschirm ist der Arbeitsplatz, und
    # ein Fenster, das sich dorthin draengt, stoert bei jeder Aufnahme.
    [ValidateSet('left', 'right', 'top', 'primary', 'none')]
    [string]$Monitor = 'left'
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

    [DllImport("user32.dll")]
    public static extern bool MoveWindow(IntPtr hWnd, int x, int y, int w, int h, bool repaint);

    [DllImport("user32.dll")]
    public static extern bool SetWindowPos(IntPtr hWnd, IntPtr after, int x, int y, int w, int h, uint flags);

    [DllImport("user32.dll")]
    public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdc, uint flags);

    // Der Weg, den auch Alt+Tab nimmt: SetForegroundWindow unterliegt der
    // Vordergrundsperre und tut aus einem Hintergrundprozess nichts,
    // SwitchToThisWindow nicht.
    [DllImport("user32.dll")]
    public static extern void SwitchToThisWindow(IntPtr hWnd, bool altTab);

    // Bezieht die von DWM zwischengespeicherte Darstellung ein. Ohne dieses
    // Flag liefert PrintWindow bei einem OpenGL-Fenster eine schwarze Flaeche.
    public const uint PW_RENDERFULLCONTENT = 0x00000002;

    public static readonly IntPtr HWND_TOPMOST = new IntPtr(-1);
    public static readonly IntPtr HWND_NOTOPMOST = new IntPtr(-2);
    public const uint SWP_SHOWWINDOW = 0x0040;
    public const uint SWP_NOACTIVATE = 0x0010;
}
'@
Add-Type -AssemblyName System.Windows.Forms

function Get-TargetScreen {
    param([string]$Which)

    $screens = [System.Windows.Forms.Screen]::AllScreens
    switch ($Which) {
        'left' { $screens | Sort-Object { $_.Bounds.X } | Select-Object -First 1 }
        'right' { $screens | Sort-Object { $_.Bounds.X } | Select-Object -Last 1 }
        'top' { $screens | Sort-Object { $_.Bounds.Y } | Select-Object -First 1 }
        'primary' { [System.Windows.Forms.Screen]::PrimaryScreen }
        default { $null }
    }
}

$exePath = (Resolve-Path $Exe).Path
$proc = Start-Process -FilePath $exePath -ArgumentList $AppArgs -PassThru

# Auf das Fenster warten statt eine Frist zu raten: MainWindowHandle bleibt
# eine Weile 0, und wie lange haengt an Scan, Treiber und Tagesform. Ein
# einmaliges Start-Sleep hat genau deshalb gelegentlich ins Leere gegriffen.
$hwnd = [IntPtr]::Zero
$deadline = (Get-Date).AddSeconds($WaitSeconds + 20)
while ((Get-Date) -lt $deadline) {
    if ($proc.HasExited) { throw "ctxmenu endete mit Code $($proc.ExitCode), bevor ein Fenster erschien" }
    $proc.Refresh()
    if ($proc.MainWindowHandle -ne [IntPtr]::Zero -and [Win]::IsWindowVisible($proc.MainWindowHandle)) {
        $hwnd = $proc.MainWindowHandle
        break
    }
    Start-Sleep -Milliseconds 300
}

try {
    if ($hwnd -eq [IntPtr]::Zero) { throw "Kein Fenster gefunden (MainWindowHandle blieb 0)" }

    $screen = Get-TargetScreen -Which $Monitor
    if ($screen) {
        $area = $screen.WorkingArea
        $ww = [Math]::Min(1400, $area.Width - 80)
        $wh = [Math]::Min(900, $area.Height - 80)
        # SetWindowPos statt MoveWindow plus SetForegroundWindow: der Aufruf
        # kommt aus einem Prozess, der nicht im Vordergrund ist, und
        # SetForegroundWindow tut dann schlicht nichts -- fotografiert wurde
        # dann das Fenster, das zufaellig darueber lag. TOPMOST hebt es ohne
        # Fokuswechsel nach oben.
        [void][Win]::SetWindowPos($hwnd, [Win]::HWND_TOPMOST, $area.X + 40, $area.Y + 40,
            $ww, $wh, [Win]::SWP_SHOWWINDOW -bor [Win]::SWP_NOACTIVATE)
        Write-Output "Fenster auf Bildschirm '$Monitor' bei $($area.X),$($area.Y)"
    }

    # Kurze Ruhe, damit der erste Scan durch ist und die Liste steht.
    Start-Sleep -Seconds $WaitSeconds

    [Win]::SwitchToThisWindow($hwnd, $true)
    [void][Win]::SetForegroundWindow($hwnd)
    Start-Sleep -Milliseconds 1200

    $rect = New-Object Win+RECT
    if (-not [Win]::GetWindowRect($hwnd, [ref]$rect)) { throw "GetWindowRect fehlgeschlagen" }

    $w = $rect.Right - $rect.Left
    $h = $rect.Bottom - $rect.Top
    # Unter 200 Punkten ist das kein Fenster, sondern ein noch nicht fertig
    # eingerichtetes: einmal nachfassen statt ein 15x15-Bild zu speichern.
    if ($w -lt 200 -or $h -lt 200) {
        Start-Sleep -Seconds 3
        if (-not [Win]::GetWindowRect($hwnd, [ref]$rect)) { throw "GetWindowRect fehlgeschlagen" }
        $w = $rect.Right - $rect.Left
        $h = $rect.Bottom - $rect.Top
    }
    if ($w -lt 200 -or $h -lt 200) { throw "Unsinnige Fenstergroesse ${w}x${h}" }

    # Erst PrintWindow: das holt die Darstellung aus dem Fenster selbst und
    # braucht es deshalb weder im Vordergrund noch unverdeckt -- wichtig, wenn
    # auf demselben Bildschirm gearbeitet wird. Je nach Treiber kommt bei einem
    # OpenGL-Fenster trotz PW_RENDERFULLCONTENT nur Schwarz zurueck; dann bleibt
    # die Bildschirmkopie.
    $bmp = New-Object System.Drawing.Bitmap($w, $h)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $hdc = $g.GetHdc()
    $printed = [Win]::PrintWindow($hwnd, $hdc, [Win]::PW_RENDERFULLCONTENT)
    $g.ReleaseHdc($hdc)

    if ($printed) {
        # Stichprobe ueber ein Raster: eine reine schwarze Flaeche heisst, dass
        # der Treiber nicht mitgespielt hat.
        $lit = 0
        for ($sx = 8; $sx -lt $w; $sx += 64) {
            for ($sy = 8; $sy -lt $h; $sy += 64) {
                $c = $bmp.GetPixel($sx, $sy)
                if ($c.R + $c.G + $c.B -gt 24) { $lit++ }
            }
        }
        if ($lit -lt 4) {
            $printed = $false
            Write-Output "PrintWindow lieferte Schwarz, weiche auf die Bildschirmkopie aus"
        }
    }

    if (-not $printed) {
        $g.CopyFromScreen($rect.Left, $rect.Top, 0, 0, (New-Object System.Drawing.Size($w, $h)))
    }
    $g.Dispose()

    $bmp.Save([System.IO.Path]::GetFullPath($Out), [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()

    Write-Output "geschrieben: $Out (${w}x${h})"
} finally {
    if (-not $proc.HasExited) { $proc | Stop-Process -Force }
}
