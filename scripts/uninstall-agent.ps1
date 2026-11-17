param([switch]$RemoveFiles)
$ErrorActionPreference = 'Stop'
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Execute como administrador.' }
$name = 'ComunicacaoLanAgent'
$service = Get-Service $name -ErrorAction SilentlyContinue
if ($service) {
    if ($service.Status -ne 'Stopped') { Stop-Service $name; $service.WaitForStatus('Stopped', [TimeSpan]::FromSeconds(15)) }
    & "$env:SystemRoot\System32\sc.exe" delete $name | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Não foi possível remover o serviço.' }
}
Unregister-ScheduledTask -TaskName 'ComunicacaoLanStation' -Confirm:$false -ErrorAction SilentlyContinue
if ($RemoveFiles) {
    Remove-Item (Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'ComunicacaoLanAgent') -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item (Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'ComunicacaoLanAgent') -Recurse -Force -ErrorAction SilentlyContinue
}
Write-Host 'Serviço removido. Use -RemoveFiles para remover também os arquivos e a identidade privada.'
