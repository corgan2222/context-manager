# Erzeugt assets/app.ico als Platzhalter, bis ein echtes Icon vorliegt.
#
# Aufbau: 16/32/48 px als 32-bpp-DIB, 256 px als PNG. Das ist die Mischung,
# die auch Visual Studio schreibt -- rc.exe und der Explorer kommen damit
# zurecht, waehrend reine PNG-Icons von aelteren rc.exe-Versionen abgelehnt
# werden.
#
# Aufruf:  pwsh -File tools\make_icon.ps1

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$OutFile = Join-Path $PSScriptRoot '..\ctxmenu\assets\app.ico'
$Sizes = @(16, 32, 48, 256)

function New-Glyph([int]$Size) {
    $bmp = New-Object System.Drawing.Bitmap($Size, $Size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    $g = [System.Drawing.Graphics]::FromImage($bmp)
    $g.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
    $g.Clear([System.Drawing.Color]::Transparent)

    # Abgerundetes Panel als Menue-Andeutung
    $pad = [Math]::Max(1, [int]($Size * 0.06))
    $r = [Math]::Max(2, [int]($Size * 0.18))
    $rect = New-Object System.Drawing.Rectangle($pad, $pad, ($Size - 2 * $pad), ($Size - 2 * $pad))

    $path = New-Object System.Drawing.Drawing2D.GraphicsPath
    $d = $r * 2
    $path.AddArc($rect.X, $rect.Y, $d, $d, 180, 90)
    $path.AddArc($rect.Right - $d, $rect.Y, $d, $d, 270, 90)
    $path.AddArc($rect.Right - $d, $rect.Bottom - $d, $d, $d, 0, 90)
    $path.AddArc($rect.X, $rect.Bottom - $d, $d, $d, 90, 90)
    $path.CloseFigure()

    $bg = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 43, 45, 52))
    $g.FillPath($bg, $path)

    # Drei Menuezeilen, die oberste im Akzentton
    $barH = [Math]::Max(1, [int]($Size * 0.10))
    $barX = [int]($Size * 0.24)
    $barW = [int]($Size * 0.52)
    $accent = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 76, 194, 255))
    $plain = New-Object System.Drawing.SolidBrush ([System.Drawing.Color]::FromArgb(255, 226, 226, 230))

    $ys = @([int]($Size * 0.28), [int]($Size * 0.45), [int]($Size * 0.62))
    for ($i = 0; $i -lt 3; $i++) {
        $brush = if ($i -eq 0) { $accent } else { $plain }
        $g.FillRectangle($brush, $barX, $ys[$i], $barW, $barH)
    }

    $bg.Dispose(); $accent.Dispose(); $plain.Dispose(); $path.Dispose(); $g.Dispose()
    return $bmp
}

function Get-PngBytes([System.Drawing.Bitmap]$Bmp) {
    $ms = New-Object System.IO.MemoryStream
    $Bmp.Save($ms, [System.Drawing.Imaging.ImageFormat]::Png)
    $bytes = $ms.ToArray()
    $ms.Dispose()
    # Komma verhindert, dass PowerShell das byte[] beim Rueckgeben entrollt --
    # sonst kommt ein Object[] an und BinaryWriter trifft die falsche Ueberladung.
    return , $bytes
}

function Get-DibBytes([System.Drawing.Bitmap]$Bmp) {
    $w = $Bmp.Width; $h = $Bmp.Height
    $ms = New-Object System.IO.MemoryStream
    $bw = New-Object System.IO.BinaryWriter($ms)

    # BITMAPINFOHEADER -- biHeight ist doppelt hoch: XOR-Bild plus AND-Maske
    $bw.Write([uint32]40); $bw.Write([int32]$w); $bw.Write([int32]($h * 2))
    $bw.Write([uint16]1); $bw.Write([uint16]32); $bw.Write([uint32]0)
    $bw.Write([uint32]($w * $h * 4))
    $bw.Write([int32]0); $bw.Write([int32]0); $bw.Write([uint32]0); $bw.Write([uint32]0)

    # XOR-Bild, BGRA, von unten nach oben
    $data = $Bmp.LockBits(
        (New-Object System.Drawing.Rectangle(0, 0, $w, $h)),
        [System.Drawing.Imaging.ImageLockMode]::ReadOnly,
        [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
    try {
        $row = New-Object byte[] ($w * 4)
        for ($y = $h - 1; $y -ge 0; $y--) {
            $ptr = [IntPtr]::Add($data.Scan0, $y * $data.Stride)
            [System.Runtime.InteropServices.Marshal]::Copy($ptr, $row, 0, $row.Length)
            $bw.Write($row)
        }
    } finally {
        $Bmp.UnlockBits($data)
    }

    # AND-Maske: bei 32 bpp traegt der Alphakanal die Transparenz, Maske bleibt 0
    $maskStride = [int]([Math]::Floor(($w + 31) / 32) * 4)
    $bw.Write((New-Object byte[] ($maskStride * $h)))

    $bw.Flush()
    $bytes = $ms.ToArray()
    $bw.Dispose(); $ms.Dispose()
    return , $bytes
}

$payloads = @()
foreach ($size in $Sizes) {
    $bmp = New-Glyph $size
    $bytes = if ($size -ge 256) { Get-PngBytes $bmp } else { Get-DibBytes $bmp }
    $payloads += , @{ Size = $size; Bytes = $bytes }
    $bmp.Dispose()
}

$ms = New-Object System.IO.MemoryStream
$bw = New-Object System.IO.BinaryWriter($ms)

# ICONDIR
$bw.Write([uint16]0); $bw.Write([uint16]1); $bw.Write([uint16]$payloads.Count)

# ICONDIRENTRY je Bild; 256 wird im Format als 0 kodiert
$offset = 6 + 16 * $payloads.Count
foreach ($p in $payloads) {
    $dim = if ($p.Size -ge 256) { 0 } else { $p.Size }
    $bw.Write([byte]$dim); $bw.Write([byte]$dim)
    $bw.Write([byte]0); $bw.Write([byte]0)
    $bw.Write([uint16]1); $bw.Write([uint16]32)
    $bw.Write([uint32]$p.Bytes.Length)
    $bw.Write([uint32]$offset)
    $offset += $p.Bytes.Length
}
foreach ($p in $payloads) { $bw.Write([byte[]]$p.Bytes) }

$bw.Flush()
[System.IO.File]::WriteAllBytes([System.IO.Path]::GetFullPath($OutFile), $ms.ToArray())
$bw.Dispose(); $ms.Dispose()

Write-Output "app.ico geschrieben: $($payloads.Count) Groessen, $((Get-Item $OutFile).Length) Bytes"
