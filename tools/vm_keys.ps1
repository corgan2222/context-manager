# Schickt Tastendruecke an die Konsole einer laufenden Hyper-V-VM.
#
# Wofuer: ein Kontextmenue laesst sich nicht ueber PowerShell Direct oeffnen —
# es entsteht erst, wenn der Explorer wirklich einen Rechtsklick sieht. Hyper-V
# stellt dafuer Msvm_Keyboard bereit, das Tastenanschlaege in die virtuelle
# Tastatur schiebt, unabhaengig davon, ob ein vmconnect-Fenster offen ist.
#
# Aufruf:
#   pwsh -File tools\vm_keys.ps1 -Keys 0x28,0x5D          # Pfeil ab, Menuetaste
#   pwsh -File tools\vm_keys.ps1 -Text 'notepad'          # Text tippen
#
# Nuetzliche Codes: TAB 0x09, ENTER 0x0D, ESC 0x1B, LEER 0x20, ENDE 0x23,
# POS1 0x24, LINKS 0x25, HOCH 0x26, RECHTS 0x27, AB 0x28, MENUE 0x5D,
# F5 0x74, WIN 0x5B.

[CmdletBinding()]
param(
    [string]$VMName = 'ctxmenu-test-win10',
    # Als Zeichenkette, kommagetrennt: bei `pwsh -File` kommt jedes Argument
    # als String an, ein [int[]]-Parameter scheitert dort an "0x28,0x5D".
    [string]$Keys,
    [string]$Text,
    # Tastenkombination, z. B. "0x5B,0x45" fuer Win+E: alle Tasten werden
    # gedrueckt gehalten und in umgekehrter Reihenfolge losgelassen. TypeKey
    # kann das nicht, es drueckt und loest sofort wieder.
    [string]$Combo,
    # Pause zwischen zwei Anschlaegen. Der Explorer verschluckt Tasten, die
    # waehrend eines Fensterwechsels ankommen.
    [int]$DelayMs = 400
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

$ns = 'root\virtualization\v2'
$vm = Get-CimInstance -Namespace $ns -ClassName Msvm_ComputerSystem -Filter "ElementName='$VMName'"
if (-not $vm) { throw "VM '$VMName' nicht gefunden" }

$keyboard = @(Get-CimAssociatedInstance -InputObject $vm -ResultClassName Msvm_Keyboard)[0]
if (-not $keyboard) { throw "Keine Msvm_Keyboard fuer '$VMName' -- laeuft die VM?" }

if ($Combo) {
    $held = $Combo -split ',' | ForEach-Object { [Convert]::ToInt32($_.Trim(), $(if ($_ -match '0[xX]') { 16 } else { 10 })) }
    foreach ($key in $held) {
        $r = Invoke-CimMethod -InputObject $keyboard -MethodName PressKey -Arguments @{ keyCode = [uint16]$key }
        if ($r.ReturnValue -ne 0) { throw ("PressKey 0x{0:X2} lieferte {1}" -f $key, $r.ReturnValue) }
        Start-Sleep -Milliseconds 60
    }
    [array]::Reverse($held)
    foreach ($key in $held) {
        $r = Invoke-CimMethod -InputObject $keyboard -MethodName ReleaseKey -Arguments @{ keyCode = [uint16]$key }
        if ($r.ReturnValue -ne 0) { throw ("ReleaseKey 0x{0:X2} lieferte {1}" -f $key, $r.ReturnValue) }
        Start-Sleep -Milliseconds 60
    }
    Write-Output "Kombination gesendet: $Combo"
    Start-Sleep -Milliseconds $DelayMs
}

if ($Text) {
    $result = Invoke-CimMethod -InputObject $keyboard -MethodName TypeText -Arguments @{ asciiText = $Text }
    if ($result.ReturnValue -ne 0) { throw "TypeText lieferte $($result.ReturnValue)" }
    Write-Output "getippt: $Text"
}

$codes = @()
if ($Keys) {
    $codes = $Keys -split ',' | ForEach-Object { [Convert]::ToInt32($_.Trim(), $(if ($_ -match '0[xX]') { 16 } else { 10 })) }
}

foreach ($key in $codes) {
    $result = Invoke-CimMethod -InputObject $keyboard -MethodName TypeKey -Arguments @{ keyCode = [uint16]$key }
    if ($result.ReturnValue -ne 0) { throw "TypeKey 0x{0:X2} lieferte $($result.ReturnValue)" -f $key }
    Write-Output ("gesendet: 0x{0:X2}" -f $key)
    Start-Sleep -Milliseconds $DelayMs
}
