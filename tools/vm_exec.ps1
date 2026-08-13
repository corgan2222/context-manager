# Fuehrt einen Befehl in der Test-VM aus, ueber PowerShell Direct.
#
# Warum ueberhaupt: die Abnahmen, die HKLM schreiben oder ein echtes
# Rechtsklickmenue zeigen sollen, duerfen laut ToDo 2.8 nicht auf dem Wirt
# laufen. PowerShell Direct braucht kein Netzwerk in der VM, nur den
# Gastdienst.
#
# Das Kennwort steht in der Antwortdatei der unbeaufsichtigten Installation
# und wird hier zur Laufzeit daraus gelesen. Es steht bewusst weder in diesem
# Skript noch in der HANDOVER.md noch in einer Befehlszeile: eine Befehlszeile
# landet in der Prozessliste und im Verlauf.
#
# Aufruf:
#   pwsh -File tools\vm_exec.ps1 -Command 'ipconfig /all'
#   pwsh -File tools\vm_exec.ps1 -ScriptFile lokal.ps1
#   pwsh -File tools\vm_exec.ps1 -CopyIn C:\pfad\ctxmenu.exe -Destination C:\ctxmenu\ctxmenu.exe

[CmdletBinding()]
param(
    [string]$VMName = 'ctxmenu-test-win10',
    [string]$AnswerFile = 'D:\temp\win10\autounattend.xml',
    # Name des Kontos aus der Antwortdatei; die VM meldet sich als
    # desktop-…\admin an.
    [string]$UserName = 'Admin',
    [string]$Command,
    [string]$ScriptFile,
    [string]$CopyIn,
    [string]$Destination
)

Set-StrictMode -Version Latest
$ErrorActionPreference = 'Stop'

function Get-VMCredential {
    param([string]$Path, [string]$User)

    if (-not (Test-Path -LiteralPath $Path)) { throw "Antwortdatei fehlt: $Path" }
    [xml]$answer = Get-Content -LiteralPath $Path -Raw

    # Die Antwortdatei traegt das Kennwort an mehreren Stellen; die erste
    # nichtleere genuegt. Base64-kodierte Werte tragen den Namen des Feldes
    # angehaengt, den Windows beim Kodieren anfuegt.
    $node = $answer.SelectSingleNode("//*[local-name()='AutoLogon']/*[local-name()='Password']/*[local-name()='Value']")
    if (-not $node) {
        $node = $answer.SelectSingleNode("//*[local-name()='LocalAccounts']//*[local-name()='Password']/*[local-name()='Value']")
    }
    if (-not $node) { throw "Kein Kennwort in $Path gefunden" }

    $raw = $node.InnerText
    $plainNode = $answer.SelectSingleNode("//*[local-name()='AutoLogon']/*[local-name()='Password']/*[local-name()='PlainText']")
    if ($plainNode -and $plainNode.InnerText -eq 'false') {
        $decoded = [Text.Encoding]::Unicode.GetString([Convert]::FromBase64String($raw))
        # Windows haengt beim Kodieren den Feldnamen an.
        $raw = $decoded -replace 'Password$', ''
    }

    $secure = ConvertTo-SecureString $raw -AsPlainText -Force
    Remove-Variable raw, decoded -ErrorAction SilentlyContinue
    New-Object System.Management.Automation.PSCredential("$User", $secure)
}

$credential = Get-VMCredential -Path $AnswerFile -User $UserName
$session = New-PSSession -VMName $VMName -Credential $credential

try {
    if ($CopyIn) {
        if (-not $Destination) { throw '-CopyIn braucht -Destination' }
        $parent = Split-Path -Parent $Destination
        Invoke-Command -Session $session -ScriptBlock {
            param($p)
            if (-not (Test-Path -LiteralPath $p)) { New-Item -ItemType Directory -Force -Path $p | Out-Null }
        } -ArgumentList $parent
        Copy-Item -LiteralPath $CopyIn -Destination $Destination -ToSession $session -Force
        Write-Output "kopiert: $CopyIn -> $Destination"
    }

    if ($ScriptFile) {
        Invoke-Command -Session $session -FilePath $ScriptFile
    }

    if ($Command) {
        Invoke-Command -Session $session -ScriptBlock ([scriptblock]::Create($Command))
    }
} finally {
    Remove-PSSession $session
}
