# Teste HTTP real, sem interface gráfica e sem alterar configurações do Windows.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$env:LAN_ADMIN_TOKEN = 'ci-admin-token-at-least-32-characters-long'
$env:LAN_STATION_TOKEN = 'ci-station-token-at-least-32-characters-long'
$env:LAN_DEMO_PAYMENTS = '1'
$env:LAN_DB = Join-Path ([IO.Path]::GetTempPath()) ('lan-ci-' + [guid]::NewGuid() + '.sqlite3')
$base = 'http://127.0.0.1:8787'
$stationHeaders = @{ Authorization = 'Bearer ' + $env:LAN_STATION_TOKEN }
$adminHeaders = @{ Authorization = 'Bearer ' + $env:LAN_ADMIN_TOKEN }
$server = $null

function Start-DemoServer {
    $process = Start-Process -FilePath '.\target\debug\lan-server.exe' -NoNewWindow -PassThru
    for ($attempt = 0; $attempt -lt 30; $attempt++) {
        $process.Refresh()
        if ($process.HasExited) { throw 'Servidor encerrou antes de responder.' }
        try {
            $null = Invoke-RestMethod "$base/api/session" -Headers $stationHeaders -TimeoutSec 2
            return $process
        } catch { Start-Sleep -Milliseconds 200 }
    }
    Stop-Process -Id $process.Id
    throw 'Servidor não ficou pronto.'
}
function Assert-Status($path, $headers, $expected) {
    try {
        $response = Invoke-WebRequest ($base + $path) -Headers $headers -Method Post -TimeoutSec 3
        $actual = [int]$response.StatusCode
    } catch {
        if (-not $_.Exception.Response) { throw }
        $actual = [int]$_.Exception.Response.StatusCode
    }
    if ($actual -ne $expected) { throw "Status inesperado: $actual (esperado $expected)." }
}
try {
    $server = Start-DemoServer
    $order = Invoke-RestMethod "$base/api/orders" -Method Post -Headers $stationHeaders -ContentType 'application/json' -Body '{"plan_id":"demo"}'
    $initial = Invoke-RestMethod "$base/api/session" -Headers $stationHeaders
    if ($initial.remaining_seconds -ne 0) { throw 'Criar pedido liberou tempo sem confirmação.' }
    $path = '/api/admin/orders/' + $order.id + '/confirm-demo'
    Assert-Status $path $stationHeaders 401
    Assert-Status $path $adminHeaders 204
    Assert-Status $path $adminHeaders 204
    Stop-Process -Id $server.Id
    $server.WaitForExit()
    $server = Start-DemoServer
    Assert-Status $path $adminHeaders 204
    $session = Invoke-RestMethod "$base/api/session" -Headers $stationHeaders
    if ($session.revision -ne 1 -or $session.remaining_seconds -le 0 -or $session.remaining_seconds -gt 90) {
        throw 'Sessão não foi preservada ou confirmação repetida duplicou crédito.'
    }
    Write-Host 'OK: compra, separação de credenciais, confirmação idempotente e reinício.'
} finally {
    if ($server) {
        $server.Refresh()
        if (-not $server.HasExited) { Stop-Process -Id $server.Id; $server.WaitForExit() }
    }
    Remove-Item $env:LAN_DB, ($env:LAN_DB + '-wal'), ($env:LAN_DB + '-shm') -ErrorAction SilentlyContinue
    Remove-Item Env:LAN_ADMIN_TOKEN, Env:LAN_STATION_TOKEN, Env:LAN_DEMO_PAYMENTS, Env:LAN_DB -ErrorAction SilentlyContinue
}
