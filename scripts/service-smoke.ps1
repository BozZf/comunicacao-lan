# CI/VM isolada: cria duas contas temporárias, instala, testa e remove o serviço.
param([switch]$WindowsGate)
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
    # Processos de teste da conta cliente não podem herdar os segredos da sessão administrativa.
    $saved = @{}
    foreach ($key in @('LAN_ADMIN_TOKEN', 'LAN_STATION_TOKEN', 'LAN_SERVER_SIGNING_KEY', 'LAN_SERVER_PUBLIC_KEY', 'LAN_DEMO_PAYMENTS', 'LAN_DB', 'LAN_WINDOWS_CLIENT_PASSWORD', 'LAN_WINDOWS_CLIENT_USER', 'LAN_WINDOWS_GATE')) {
        $saved[$key] = [Environment]::GetEnvironmentVariable($key, 'Process')
        [Environment]::SetEnvironmentVariable($key, $null, 'Process')
    }
    try {
        $process = Start-Process -FilePath $Executable -ArgumentList $Arguments -WorkingDirectory $installDir `
            -Credential $UserCredential -RedirectStandardOutput $out -RedirectStandardError $err -Wait -PassThru
    } finally {
        foreach ($key in $saved.Keys) { [Environment]::SetEnvironmentVariable($key, $saved[$key], 'Process') }
    }
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
    if ($WindowsGate) {
        $env:LAN_WINDOWS_GATE = '1'
        $env:LAN_WINDOWS_CLIENT_USER = $clientUser
        $bstr = [Runtime.InteropServices.Marshal]::SecureStringToBSTR($password)
        try { $env:LAN_WINDOWS_CLIENT_PASSWORD = [Runtime.InteropServices.Marshal]::PtrToStringBSTR($bstr) }
        finally { [Runtime.InteropServices.Marshal]::ZeroFreeBSTR($bstr) }
    }
    & './scripts/install-agent.ps1' -ClientUser $clientUser -WindowsGate:$WindowsGate
    Remove-Item Env:LAN_WINDOWS_CLIENT_PASSWORD -ErrorAction SilentlyContinue
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
    $deniedCredentials = Run-AsClient $agent @('--probe', '--check-logon-denied') $credential
    if ($deniedCredentials.ExitCode -ne 0) { throw 'Serialização reservada ao Windows acessível ao cliente.' }
    if ($WindowsGate) {
        $taskName = 'LanAuthTest-' + $suffix
        try {
            $privatePath = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'ComunicacaoLanAgent\private.json'
            $taskAction = New-ScheduledTaskAction -Execute $agent -Argument ('--self-test-logon "' + $privatePath + '"')
            $taskPrincipal = New-ScheduledTaskPrincipal -UserId 'S-1-5-18' -LogonType ServiceAccount -RunLevel Highest
            $null = Register-ScheduledTask -TaskName $taskName -Action $taskAction -Principal $taskPrincipal
            $started = Get-Date
            Start-ScheduledTask -TaskName $taskName
            $done = $false
            for ($i = 0; $i -lt 100; $i++) {
                Start-Sleep -Milliseconds 300
                $info = Get-ScheduledTaskInfo -TaskName $taskName
                if ($info.LastRunTime -ge $started.AddSeconds(-2) -and (Get-ScheduledTask -TaskName $taskName).State -ne 'Running') {
                    if ($info.LastTaskResult -ne 0) { throw ('Serialização não autenticou no Windows: código ' + $info.LastTaskResult) }
                    $done = $true
                    break
                }
            }
            if (-not $done) { throw 'Diagnóstico de autenticação não terminou.' }
        } finally { Unregister-ScheduledTask -TaskName $taskName -Confirm:$false -ErrorAction SilentlyContinue }
    }
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
    Remove-Item Env:LAN_ADMIN_TOKEN, Env:LAN_STATION_TOKEN, Env:LAN_DEMO_PAYMENTS, Env:LAN_DB, Env:LAN_SERVER_SIGNING_KEY, Env:LAN_SERVER_PUBLIC_KEY, Env:LAN_WINDOWS_GATE, Env:LAN_WINDOWS_CLIENT_USER, Env:LAN_WINDOWS_CLIENT_PASSWORD -ErrorAction SilentlyContinue
}
