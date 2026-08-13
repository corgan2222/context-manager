# Holt ein Konsolenbild einer laufenden Hyper-V-VM, ohne vmconnect zu oeffnen.
#
# Zweck: eine unbeaufsichtigte Installation laesst sich sonst nur an der
# Uhr ablesen. Hyper-V liefert ueber WMI ein Vorschaubild des Bildschirms;
# damit ist der Fortschritt tatsaechlich sichtbar.
#
# Das Bild kommt als RGB565 zurueck, zwei Bytes je Bildpunkt, und muss von
# Hand in eine Bitmap uebersetzt werden.
#
# Aufruf:  pwsh -File tools\vm_thumbnail.ps1 -VMName ctxmenu-test-win10 -Out shot.png

param(
    [Parameter(Mandatory)] [string]$VMName,
    [string]$Out = 'vm.png',
    [int]$Width = 800,
    [int]$Height = 600
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'
Add-Type -AssemblyName System.Drawing

$ns = 'root\virtualization\v2'

$vm = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$VMName'"
if (-not $vm) { throw "VM '$VMName' nicht gefunden" }

$settings = @(Get-CimAssociatedInstance -InputObject $vm `
        -ResultClassName Msvm_VirtualSystemSettingData -Association Msvm_SettingsDefineState)[0]
if (-not $settings) { throw "Keine VirtualSystemSettingData fuer '$VMName'" }

$service = Get-CimInstance -Namespace $ns -ClassName Msvm_VirtualSystemManagementService

# Referenzparameter werden bei Invoke-CimMethod als CimInstance uebergeben,
# NICHT als [ref] -- sonst kommt ein Cast-Fehler auf InstanceHandle.
$result = Invoke-CimMethod -InputObject $service -MethodName GetVirtualSystemThumbnailImage -Arguments @{
    TargetSystem = $settings
    WidthPixels  = [uint16]$Width
    HeightPixels = [uint16]$Height
}

if ($result.ReturnValue -ne 0) { throw "GetVirtualSystemThumbnailImage lieferte $($result.ReturnValue)" }

$data = $result.ImageData
if (-not $data -or $data.Length -lt ($Width * $Height * 2)) {
    throw "Bilddaten unvollstaendig: $($data.Length) Bytes fuer ${Width}x${Height}"
}

$bmp = New-Object System.Drawing.Bitmap($Width, $Height, [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
$rect = New-Object System.Drawing.Rectangle(0, 0, $Width, $Height)
$locked = $bmp.LockBits($rect, [System.Drawing.Imaging.ImageLockMode]::WriteOnly,
    [System.Drawing.Imaging.PixelFormat]::Format24bppRgb)
try {
    $row = New-Object byte[] $locked.Stride
    for ($y = 0; $y -lt $Height; $y++) {
        for ($x = 0; $x -lt $Width; $x++) {
            $i = ($y * $Width + $x) * 2
            $v = [int]$data[$i] -bor ([int]$data[$i + 1] -shl 8)
            # RGB565 auf je acht Bit strecken: die oberen Bits wiederholen,
            # sonst erreicht Weiss nie 255.
            $r = (($v -shr 11) -band 0x1F); $r = ($r -shl 3) -bor ($r -shr 2)
            $g = (($v -shr 5) -band 0x3F);  $g = ($g -shl 2) -bor ($g -shr 4)
            $b = ($v -band 0x1F);           $b = ($b -shl 3) -bor ($b -shr 2)
            $o = $x * 3
            $row[$o] = [byte]$b      # Format24bppRgb liegt als BGR im Speicher
            $row[$o + 1] = [byte]$g
            $row[$o + 2] = [byte]$r
        }
        [System.Runtime.InteropServices.Marshal]::Copy($row, 0,
            [IntPtr]::Add($locked.Scan0, $y * $locked.Stride), $locked.Stride)
    }
} finally {
    $bmp.UnlockBits($locked)
}

$bmp.Save([System.IO.Path]::GetFullPath($Out), [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
Write-Output "geschrieben: $Out (${Width}x${Height})"
