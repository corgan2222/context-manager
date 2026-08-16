# Legt eine Test-VM an, in der die HKLM-Schreibtests laufen.
#
# Vor der ersten Schreiboperation braucht es eine VM mit Checkpoint:
# ein Fehler in HKLM\SOFTWARE\Classes macht den Explorer unbenutzbar, und das
# faellt erst beim naechsten Rechtsklick auf.
#
# Seit dem 2026-08-16 auch fuer Windows 11: -Tpm schaltet den virtuellen TPM
# und den Schluesselschutz ein, ohne die Setup sich weigert zu installieren.
#
# Voraussetzungen:
#   - Hyper-V aktiviert, Konto in der Gruppe "Hyper-V-Administratoren"
#   - der Dienst vmms laeuft (Start-Service vmms; die Dienst-ACL erlaubt das
#     dieser Gruppe ausdruecklich, eine Erhoehung ist nicht noetig)
#   - eine unbeaufsichtigte ISO
#
# Aufruf:
#   pwsh -File tools\vm_setup.ps1 -Iso D:\temp\win10\Win10_22H2_unattend.iso
#   pwsh -File tools\vm_setup.ps1 -Iso D:\Temp\Win11_25H2_unraid_en.iso `
#        -Name ctxmenu-test-win11 -Tpm `
#        -VhdPath 'D:\Hyper-V\ctxmenu-test-win11.vhdx'

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
    [int]$Cpu = 4,
    # Windows 11 prueft TPM 2.0 und Secure Boot, bevor es ueberhaupt anfaengt,
    # und bricht sonst mit "Auf diesem PC kann Windows 11 nicht ausgefuehrt
    # werden" ab. Hyper-V bringt beides mit; der Schluesselschutz muss vorher
    # angelegt werden, sonst laesst sich der vTPM nicht einschalten.
    [switch]$Tpm
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

if ($Tpm) {
    # Reihenfolge ist Pflicht: ohne Schluesselschutz weigert sich
    # Enable-VMTpm mit "Der Schluesselschutz fuer die VM ist ungueltig".
    Write-Host 'Richte den virtuellen TPM ein ...'
    # Unbedingt, nicht nur wenn keiner da ist: eine frische VM hat bereits
    # einen Schluesselschutz, nur einen leeren, und `Get-VMKeyProtector`
    # liefert dafuer keinen Null-Wert. Die Pruefung sah also erfuellt aus,
    # und Enable-VMTpm scheiterte an genau der Vorbedingung, die sie
    # herstellen sollte.
    Set-VMKeyProtector -VMName $Name -NewLocalKeyProtector
    Enable-VMTpm -VMName $Name
    $tpm = Get-VMSecurity -VMName $Name
    Write-Host "  TPM aktiviert: $($tpm.TpmEnabled), Verschluesselung: $($tpm.EncryptStateAndVmMigrationTraffic)"
}

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
