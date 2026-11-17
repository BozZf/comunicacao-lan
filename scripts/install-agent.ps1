# Instalação EXPERIMENTAL. Execute como administrador somente em uma VM de teste.
# Não cria bloqueio de acesso ao desktop e não altera UAC ou Ctrl+Alt+Del.
param(
    [Parameter(Mandatory = $true)][string]$ClientUser,
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\target\debug'),
    [switch]$StartInterfaceAtLogon
)
$ErrorActionPreference = 'Stop'
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Execute o instalador em um terminal como administrador, em uma VM.' }
if (-not $env:LAN_STATION_TOKEN -or -not $env:LAN_SERVER_PUBLIC_KEY) { throw 'Defina LAN_STATION_TOKEN e LAN_SERVER_PUBLIC_KEY antes de instalar.' }
$client = Get-LocalUser -Name $ClientUser
if (-not $client.Enabled) { throw 'A conta de cliente deve estar habilitada.' }
$adminSids = @(Get-LocalGroupMember -SID 'S-1-5-32-544' | ForEach-Object { $_.SID.Value })
if ($adminSids -contains $client.SID.Value) { throw 'Use uma conta local padrão, fora do grupo Administradores.' }
$name = 'ComunicacaoLanAgent'
$installDir = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'ComunicacaoLanAgent'
$privateDir = Join-Path ([Environment]::GetFolderPath('CommonApplicationData')) 'ComunicacaoLanAgent'
$privatePath = Join-Path $privateDir 'private.json'
if ((Get-Service -Name $name -ErrorAction SilentlyContinue) -or (Test-Path $installDir) -or (Test-Path $privateDir)) {
    throw 'Já existe uma instalação ou diretório reservado. Não será sobrescrito.'
}
foreach ($binary in @('lan-agent.exe', 'lan-station.exe')) {
    if (-not (Test-Path (Join-Path $BuildDirectory $binary))) { throw "Executável ausente: $binary. Compile antes de instalar." }
}
function Invoke-Sc {
    param([Parameter(ValueFromRemainingArguments = $true)][string[]]$Arguments)
    & "$env:SystemRoot\System32\sc.exe" @Arguments | Out-Null
    if ($LASTEXITCODE -ne 0) { throw ('Falha ao configurar serviço: ' + $Arguments[0]) }
}
function Set-DirectoryAcl($Path, $Entries) {
    & icacls.exe $Path '/inheritance:r' '/grant:r' @Entries | Out-Null
    if ($LASTEXITCODE -ne 0) { throw 'Falha ao proteger diretório.' }
}
$createdService = $false
try {
    $null = New-Item $installDir -ItemType Directory
    Set-DirectoryAcl $installDir @('*S-1-5-18:(OI)(CI)(F)', '*S-1-5-32-544:(OI)(CI)(F)', '*S-1-5-32-545:(OI)(CI)(RX)')
    $null = New-Item $privateDir -ItemType Directory
    Set-DirectoryAcl $privateDir @('*S-1-5-18:(OI)(CI)(F)', '*S-1-5-32-544:(OI)(CI)(F)')
    Copy-Item (Join-Path $BuildDirectory 'lan-agent.exe') $installDir
    Copy-Item (Join-Path $BuildDirectory 'lan-station.exe') $installDir
    $agent = Join-Path $installDir 'lan-agent.exe'
    $binaryPath = '"' + $agent + '" --service "' + $privatePath + '"'
    Invoke-Sc create $name 'binPath=' $binaryPath 'start=' auto 'obj=' 'NT AUTHORITY\LocalService' 'DisplayName=' 'Comunicação LAN - Agente experimental'
    $createdService = $true
    Invoke-Sc sidtype $name unrestricted
    $sidOutput = & "$env:SystemRoot\System32\sc.exe" showsid $name
    if ($LASTEXITCODE -ne 0) { throw 'Não foi possível resolver o SID do serviço.' }
    $serviceSid = [regex]::Match(($sidOutput -join ' '), 'S-1-5-80-(?:\d+-){4}\d+').Value
    if (-not $serviceSid) { throw 'SID do serviço inválido.' }
    Set-DirectoryAcl $privateDir @('*S-1-5-18:(OI)(CI)(F)', '*S-1-5-32-544:(OI)(CI)(F)', ("*${serviceSid}:(OI)(CI)(RX)"))
    $env:LAN_ALLOWED_CLIENT_SID = $client.SID.Value
    $env:LAN_SERVICE_SID = $serviceSid
    & $agent --provision $privatePath (Join-Path $installDir 'agent-public.json')
    if ($LASTEXITCODE -ne 0) { throw 'Falha ao provisionar a identidade do agente.' }
    # Usuários autenticados podem consultar estado, mas não parar/configurar o serviço.
    Invoke-Sc sdset $name 'D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;CCLCLO;;;AU)'
    Invoke-Sc failure $name 'reset=' 86400 'actions=' 'restart/5000/restart/15000/restart/60000'
    Invoke-Sc failureflag $name 1
    Start-Service $name
    (Get-Service $name).WaitForStatus('Running', [TimeSpan]::FromSeconds(15))
    if ($StartInterfaceAtLogon) {
        $action = New-ScheduledTaskAction -Execute (Join-Path $installDir 'lan-station.exe') -Argument '--agent'
        $trigger = New-ScheduledTaskTrigger -AtLogOn -User $client.SID.Value
        $taskPrincipal = New-ScheduledTaskPrincipal -UserId $client.SID.Value -LogonType Interactive -RunLevel Limited
        $null = Register-ScheduledTask -TaskName 'ComunicacaoLanStation' -Action $action -Trigger $trigger -Principal $taskPrincipal
    }
    Write-Host 'Agente experimental instalado como LocalService, com credenciais protegidas.'
    Write-Host ('Interface (execute na conta de cliente): ' + (Join-Path $installDir 'lan-station.exe'))
    Write-Host 'A proteção de acesso ao desktop ainda não está implementada.'
} catch {
    if ($createdService) {
        Stop-Service $name -ErrorAction SilentlyContinue
        & "$env:SystemRoot\System32\sc.exe" delete $name | Out-Null
    }
    Unregister-ScheduledTask -TaskName 'ComunicacaoLanStation' -Confirm:$false -ErrorAction SilentlyContinue
    Remove-Item $installDir, $privateDir -Recurse -Force -ErrorAction SilentlyContinue
    throw
} finally {
    Remove-Item Env:LAN_ALLOWED_CLIENT_SID, Env:LAN_SERVICE_SID -ErrorAction SilentlyContinue
}
