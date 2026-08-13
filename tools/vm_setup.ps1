# Legt die Windows-10-Test-VM an, in der die HKLM-Schreibtests laufen.
#
# ToDo 2.8 verlangt eine VM mit Checkpoint vor der ersten Schreiboperation:
# ein Fehler in HKLM\SOFTWARE\Classes macht den Explorer unbenutzbar, und das
# faellt erst beim naechsten Rechtsklick auf.
#
# Voraussetzungen:
#   - Hyper-V aktiviert, Konto in der Gruppe "Hyper-V-Administratoren"
#   - der Dienst vmms laeuft (Start-Service vmms; die Dienst-ACL erlaubt das
#     dieser Gruppe ausdruecklich, eine Erhoehung ist nicht noetig)
#   - eine unbeaufsichtigte Windows-10-ISO
#
# Aufruf:
#   pwsh -File tools\vm_setup.ps1 -Iso D:\temp\win10\Win10_22H2_unattend.iso

[CmdletBinding()]
param(
    [Parameter(Mandatory)] [string]$Iso,
    [string]$Name = 'ctxmenu-test-win10',
    [string]$VhdPath = "$env:ProgramData\Microsoft\Windows\Virtual Hard Disks\ctxmenu-test-win10.vhdx",
    [string]$SwitchName = 'Default Switch',
    # Die autounattend.xml prueft, dass Datenträger 0 mindestens 40 GiB gross
    # ist und KEINE Partition traegt, sonst haelt Setup an. 64 GB erfuellt das
    # und laesst Platz fuer Testprogramme.
    [uint64]$DiskBytes = 64GB,
    [uint64]$MemoryBytes = 4GB,
    [int]$Cpu = 4
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

if (-not (Test-Path -LiteralPath $Iso)) { throw "ISO nicht gefunden: $Iso" }
if ((Get-Service vmms).Status -ne 'Running') {
    Write-Host 'Starte vmms ...'
    Start-Service vmms
}
if (Get-VM -Name $Name -ErrorAction SilentlyContinue) { throw "VM '$Name' existiert bereits" }

Write-Host "Lege VHDX an: $VhdPath"
New-VHD -Path $VhdPath -SizeBytes $DiskBytes -Dynamic | Out-Null

# Generation 2 = UEFI. Die autounattend erkennt die Firmware selbst und waehlt
# GPT oder MBR, Generation 1 ginge also auch.
Write-Host "Lege VM an: $Name"
New-VM -Name $Name -MemoryStartupBytes $MemoryBytes -Generation 2 -VHDPath $VhdPath -SwitchName $SwitchName | Out-Null
Set-VMProcessor -VMName $Name -Count $Cpu
Set-VMMemory -VMName $Name -DynamicMemoryEnabled $true -MinimumBytes 2GB -MaximumBytes 8GB

# Automatische Checkpoints aus: sie wuerden waehrend der Tests unangekuendigt
# Snapshots anlegen und die Platte fuellen. Checkpoints setzt dieses Projekt
# absichtlich von Hand, vor jeder Schreibrunde.
Set-VM -Name $Name -AutomaticCheckpointsEnabled $false -CheckpointType Standard `
    -AutomaticStartAction Nothing -AutomaticStopAction ShutDown

# Gastdienstschnittstelle: erlaubt spaeter Copy-VMFile, also das Einspielen
# von ctxmenu.exe ohne Netzwerkfreigabe.
Get-VMIntegrationService -VMName $Name | Enable-VMIntegrationService -ErrorAction SilentlyContinue

Add-VMDvdDrive -VMName $Name -Path $Iso
Set-VMFirmware -VMName $Name -FirstBootDevice (Get-VMDvdDrive -VMName $Name)
Set-VMFirmware -VMName $Name -EnableSecureBoot On -SecureBootTemplate MicrosoftWindows

Write-Host 'Starte VM ...'
Start-VM -Name $Name

# Eine mit efisys.bin gebaute ISO verlangt "Press any key to boot from CD or
# DVD". Ohne Tastendruck faellt die UEFI-Firmware durch und meldet lapidar
# "The boot loader failed". Deshalb wird hier fuer 25 s die Leertaste
# geschickt. Wird die ISO mit efisys_noprompt.bin gebaut, entfaellt das.
Write-Host 'Sende Leertaste (Bootmedium-Abfrage) ...'
$ns = 'root\virtualization\v2'
$deadline = (Get-Date).AddSeconds(25)
while ((Get-Date) -lt $deadline) {
    try {
        $cim = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$Name'"
        $kb = @(Get-CimAssociatedInstance -InputObject $cim -ResultClassName Msvm_Keyboard)[0]
        if ($kb) { $null = Invoke-CimMethod -InputObject $kb -MethodName TypeKey -Arguments @{ keyCode = [uint32]0x20 } }
    } catch { }
    Start-Sleep -Milliseconds 400
}

Write-Host ''
Write-Host "VM '$Name' laeuft. Die Installation ist unbeaufsichtigt und dauert rund 15 bis 25 Minuten."
Write-Host 'Fortschritt ohne vmconnect ansehen:'
Write-Host "  pwsh -File tools\vm_thumbnail.ps1 -VMName $Name -Out vm.png"
Write-Host ''
Write-Host 'Danach ZUERST einen Checkpoint setzen, vor jeder Schreiboperation:'
Write-Host "  Checkpoint-VM -Name $Name -SnapshotName 'frisch installiert'"
