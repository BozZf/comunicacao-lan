# CI/VM isolada: cria duas contas temporárias, instala, testa e remove o serviço.
$ErrorActionPreference = 'Stop'
Set-Location (Join-Path $PSScriptRoot '..')
$suffix = [guid]::NewGuid().ToString('N').Substring(0, 6)
$clientUser = 'LanTest' + $suffix
$otherUser = 'LanOther' + $suffix
$password = ConvertTo-SecureString ('Lan!-' + [guid]::NewGuid().ToString('N')) -AsPlainText -Force
$credential = New-Object Management.Automation.PSCredential("$env:COMPUTERNAME\$clientUser", $password)
$otherCredential = New-Object Management.Automation.PSCredential("$env:COMPUTERNAME\$otherUser", $password)
$env:LAN_ADMIN_TOKEN = 'ci-admin-token-at-least-32-characters-long'
$env:LAN_STATION_TOKEN = 'ci-agent-token-at-least-32-characters-long'
$env:LAN_DEMO_PAYMENTS = '1'
$env:LAN_DB = Join-Path ([IO.Path]::GetTempPath()) ('lan-service-' + $suffix + '.sqlite3')
$work = Join-Path ([IO.Path]::GetTempPath()) ('lan-probe-' + $suffix)
$null = New-Item $work -ItemType Directory
$installDir = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'ComunicacaoLanAgent'
$agent = Join-Path $installDir 'lan-agent.exe'
$server = $null
$installed = $false
$base = 'http://127.0.0.1:8787'
$adminHeaders = @{ Authorization = 'Bearer ' + $env:LAN_ADMIN_TOKEN }
$stationHeaders = @{ Authorization = 'Bearer ' + $env:LAN_STATION_TOKEN }

function Start-Backend {
    $process = Start-Process '.\target\debug\lan-server.exe' -NoNewWindow -PassThru
    for ($i = 0; $i -lt 30; $i++) {
        $process.Refresh()
        if ($process.HasExited) { throw 'Backend encerrou.' }
        try { $null = Invoke-RestMethod "$base/api/session" -Headers $stationHeaders -TimeoutSec 2; return $process }
        catch { Start-Sleep -Milliseconds 200 }
    }
    Stop-Process $process.Id
    throw 'Backend não iniciou.'
}
function Run-AsClient($Executable, $Arguments, $UserCredential) {
    $out = Join-Path $work ([guid]::NewGuid().ToString('N') + '.out')
    $err = $out + '.err'
    $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WorkingDirectory $installDir `
        -Credential $UserCredential -RedirectStandardOutput $out -RedirectStandardError $err -Wait -PassThru
    return @{ ExitCode = $process.ExitCode; Output = (Get-Content $out -Raw -ErrorAction SilentlyContinue); Error = (Get-Content $err -Raw -ErrorAction SilentlyContinue) }
}
function Probe {
    $result = Run-AsClient $agent @('--probe') $credential
    if ($result.ExitCode -ne 0) { throw ('Probe recusado: ' + $result.Error) }
    return ($result.Output | ConvertFrom-Json)
}
function Wait-Online {
    for ($i = 0; $i -lt 20; $i++) {
        try { $status = Probe; if ($status.online) { return $status } } catch { }
        Start-Sleep -Milliseconds 400
    }
    throw 'Agente não recebeu estado assinado do backend.'
}
try {
    $null = New-LocalUser -Name $clientUser -Password $password
    $null = New-LocalUser -Name $otherUser -Password $password
    # Get-LocalUser/New-LocalUser não adicionam automaticamente ao grupo Users.
    Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $clientUser, $otherUser
    $keys = & '.\target\debug\lan-server.exe' --generate-agent-key
    if ($LASTEXITCODE -ne 0) { throw 'Não foi possível gerar identidade do backend.' }
    foreach ($line in $keys) {
        $parts = $line.Split('=', 2)
        [Environment]::SetEnvironmentVariable($parts[0], $parts[1], 'Process')
    }
    $server = Start-Backend
    & './scripts/install-agent.ps1' -ClientUser $clientUser
    $installed = $true
    $status = Wait-Online
    if ($status.session.remaining_seconds -ne 0) { throw 'Agente iniciou com crédito indevido.' }
    $denied = Run-AsClient $agent @('--probe') $otherCredential
    if ($denied.ExitCode -eq 0) { throw 'Outra conta conseguiu acessar o pipe.' }
    $private = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'ComunicacaoLanAgent\private.json'
    $code = "try { [IO.File]::ReadAllText('$private') | Out-Null; exit 8 } catch { exit 0 }"
    $encoded = [Convert]::ToBase64String([Text.Encoding]::Unicode.GetBytes($code))
    $read = Run-AsClient "$env:SystemRoot\System32\WindowsPowerShell\v1.0\powershell.exe" @('-NoProfile', '-NonInteractive', '-EncodedCommand', $encoded) $credential
    if ($read.ExitCode -ne 0) { throw 'A conta de cliente conseguiu ler a configuração privada.' }
    $stop = Run-AsClient "$env:SystemRoot\System32\sc.exe" @('stop', 'ComunicacaoLanAgent') $credential
    if ($stop.ExitCode -eq 0) { throw 'Conta de cliente conseguiu parar o serviço.' }
    $created = Run-AsClient $agent @('--probe', '--buy', '1h') $credential
    if ($created.ExitCode -ne 0 -or ($created.Output | ConvertFrom-Json).type -ne 'order_created') { throw 'Compra via agente falhou.' }
    $overview = Invoke-RestMethod "$base/api/admin/overview" -Headers $adminHeaders
    $order = $overview.session.pending_order
    if (-not $order) { throw 'Cobrança não foi criada no servidor.' }
    $null = Invoke-WebRequest "$base/api/admin/orders/$($order.id)/confirm-demo" -Headers $adminHeaders -Method Post
    for ($i = 0; $i -lt 15; $i++) {
        $status = Probe
        if ($status.session.revision -eq 1) { break }
        Start-Sleep -Milliseconds 300
    }
    if ($status.session.revision -ne 1 -or $status.session.remaining_seconds -le 0) { throw 'Pagamento não chegou ao serviço.' }
    # Cada probe é um processo novo. O prazo pertence ao serviço, não à interface.
    $before = $status.session.remaining_seconds
    Stop-Process -Id $server.Id
    $server.WaitForExit()
    Start-Sleep -Seconds 3
    $offline = Probe
    if ($offline.online -or $offline.session.remaining_seconds -ge $before -or $offline.session.remaining_seconds -le 0) { throw 'Contagem offline incorreta.' }
    $server = Start-Backend
    $null = Wait-Online
    # Falha abrupta do agente: o SCM deve reiniciar e recuperar o prazo do servidor.
    $serviceProcess = Get-CimInstance Win32_Service -Filter "Name='ComunicacaoLanAgent'"
    Stop-Process -Id $serviceProcess.ProcessId -Force
    $recovered = Wait-Online
    if ($recovered.session.revision -ne 1 -or $recovered.session.remaining_seconds -gt $before) { throw 'Recuperação duplicou ou perdeu a revisão da sessão.' }
    Write-Host 'OK: SCM/LocalService, ACL do pipe, segredo privado, RPC assinado, compra, contagem offline e recuperação após crash.'
} finally {
    if ($installed) { & './scripts/uninstall-agent.ps1' -RemoveFiles }
    if ($server) { $server.Refresh(); if (-not $server.HasExited) { Stop-Process -Id $server.Id; $server.WaitForExit() } }
    Remove-LocalUser -Name $clientUser -ErrorAction SilentlyContinue
    Remove-LocalUser -Name $otherUser -ErrorAction SilentlyContinue
    Remove-Item $work -Recurse -Force -ErrorAction SilentlyContinue
    Remove-Item $env:LAN_DB, ($env:LAN_DB + '-wal'), ($env:LAN_DB + '-shm') -ErrorAction SilentlyContinue
    Remove-Item Env:LAN_ADMIN_TOKEN, Env:LAN_STATION_TOKEN, Env:LAN_DEMO_PAYMENTS, Env:LAN_DB, Env:LAN_SERVER_SIGNING_KEY, Env:LAN_SERVER_PUBLIC_KEY -ErrorAction SilentlyContinue
}
