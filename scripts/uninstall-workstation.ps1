param([switch]$RemoveFiles, [switch]$RemoveClientAccount)
$ErrorActionPreference = 'Stop'
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Execute como administrador.' }
$class = '{E463AE81-2BA8-4FD3-948A-4BAF1809818E}'
$installDir = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'ComunicacaoLanAgent'
$manifestPath = Join-Path $installDir 'workstation.json'
$manifest = if (Test-Path $manifestPath) { Get-Content $manifestPath -Raw | ConvertFrom-Json } else { $null }
# Somente as duas chaves pertencentes a este provedor; nenhum filtro/sistema é alterado.
Remove-Item ('HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\' + $class) -Recurse -Force -ErrorAction SilentlyContinue
Remove-Item ('HKLM:\SOFTWARE\Classes\CLSID\' + $class) -Recurse -Force -ErrorAction SilentlyContinue
if ($manifest) {
    $client = Get-LocalUser -Name $manifest.client_user -ErrorAction SilentlyContinue
    if ($client -and $client.SID.Value -eq $manifest.client_sid) {
        if ($RemoveClientAccount) { Remove-LocalUser -Name $client.Name }
        else { Disable-LocalUser -Name $client.Name }
    }
}
& (Join-Path $PSScriptRoot 'uninstall-agent.ps1') -RemoveFiles:$RemoveFiles
Write-Host 'Provedor removido. A conta dedicada foi desativada ou removida conforme a opção selecionada.'
