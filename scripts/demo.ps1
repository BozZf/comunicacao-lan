# Executar em um terminal PowerShell, sem privilégios de administrador.
param([switch]$SkipBuild)
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')

function New-DemoToken {
    $bytes = New-Object byte[] 32
    $rng = [System.Security.Cryptography.RandomNumberGenerator]::Create()
    try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
    return ([BitConverter]::ToString($bytes)).Replace('-', '').ToLowerInvariant()
}

if (-not $SkipBuild) {
    if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
        throw 'Instale Rust com o toolchain MSVC e as ferramentas de compilação C++ antes de executar.'
    }
    cargo build -p lan-server -p lan-station
    if ($LASTEXITCODE -ne 0) { throw 'Falha na compilação. Nenhum aplicativo foi iniciado.' }
}

$env:LAN_ADMIN_TOKEN = New-DemoToken
$env:LAN_STATION_TOKEN = New-DemoToken
$env:LAN_DEMO_PAYMENTS = '1'
$env:LAN_DB = Join-Path (Get-Location) 'lan-demo.sqlite3'
Write-Host 'Protótipo de um PC. Nenhum Pix real ou proteção do Windows.'
Write-Host 'Token do atendente (cole no painel local):'
Write-Host $env:LAN_ADMIN_TOKEN
Write-Host 'Painel: http://127.0.0.1:8787/'

$server = Start-Process -FilePath '.\target\debug\lan-server.exe' -PassThru -NoNewWindow
Remove-Item Env:LAN_ADMIN_TOKEN
try {
    # Só abre a estação se este processo conseguiu manter o servidor ativo.
    Start-Sleep -Milliseconds 800
    $server.Refresh()
    if ($server.HasExited) { throw 'O servidor encerrou. Verifique se a porta 8787 já está em uso.' }
    Start-Process 'http://127.0.0.1:8787/'
    & '.\target\debug\lan-station.exe' --demo
} finally {
    if (-not $server.HasExited) { Stop-Process -Id $server.Id }
    Remove-Item Env:LAN_ADMIN_TOKEN, Env:LAN_STATION_TOKEN, Env:LAN_DEMO_PAYMENTS -ErrorAction SilentlyContinue
}
