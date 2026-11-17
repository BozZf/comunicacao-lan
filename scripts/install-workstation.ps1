# Instalação do protótipo de bloqueio do Windows. Teste primeiro em uma VM Windows Pro.
# Cria uma conta padrão NOVA e um provedor de entrada ADICIONAL. Não modifica provedores Microsoft.
param(
    [string]$ClientUser = 'LanCliente',
    [string]$BuildDirectory = (Join-Path $PSScriptRoot '..\target\debug')
)
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Use PowerShell de 64 bits no Windows 10/11 Pro x64.' }
$principal = New-Object Security.Principal.WindowsPrincipal([Security.Principal.WindowsIdentity]::GetCurrent())
if (-not $principal.IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) { throw 'Execute como administrador na VM Windows Pro.' }
if ($ClientUser -notmatch '^[a-zA-Z0-9_-]{1,20}$') { throw 'Use um nome local simples de até 20 caracteres.' }
if (Get-LocalUser -Name $ClientUser -ErrorAction SilentlyContinue) { throw 'A conta já existe. Este instalador não altera a senha de contas existentes.' }
$class = '{E463AE81-2BA8-4FD3-948A-4BAF1809818E}'
$classPath = 'HKLM:\SOFTWARE\Classes\CLSID\' + $class
$providerPath = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\' + $class
if ((Test-Path $classPath) -or (Test-Path $providerPath)) { throw 'O provedor já está registrado.' }
foreach ($binary in @('lan-agent.exe', 'lan-station.exe', 'lan_logon.dll')) {
    if (-not (Test-Path (Join-Path $BuildDirectory $binary))) { throw "Compile os executáveis e a DLL antes de instalar: $binary" }
}
if (-not $env:LAN_STATION_TOKEN -or -not $env:LAN_SERVER_PUBLIC_KEY) { throw 'Configure o backend e defina LAN_STATION_TOKEN e LAN_SERVER_PUBLIC_KEY.' }
$bytes = New-Object byte[] 48
$rng = [Security.Cryptography.RandomNumberGenerator]::Create()
try { $rng.GetBytes($bytes) } finally { $rng.Dispose() }
$clear = [Convert]::ToBase64String($bytes)
$secure = ConvertTo-SecureString $clear -AsPlainText -Force
$created = $false
$installed = $false
$installDir = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'ComunicacaoLanAgent'
try {
    $client = New-LocalUser -Name $ClientUser -Password $secure -PasswordNeverExpires -UserMayNotChangePassword -Description 'Conta dedicada do controle de tempo Comunicação LAN'
    $created = $true
    Add-LocalGroupMember -SID 'S-1-5-32-545' -Member $ClientUser
    $env:LAN_WINDOWS_GATE = '1'
    $env:LAN_WINDOWS_CLIENT_USER = $ClientUser
    $env:LAN_WINDOWS_CLIENT_PASSWORD = $clear
    & (Join-Path $PSScriptRoot 'install-agent.ps1') -ClientUser $ClientUser -BuildDirectory $BuildDirectory -WindowsGate -StartInterfaceAtLogon
    $installed = $true
    $manifest = @{ client_user = $ClientUser; client_sid = $client.SID.Value; provider_clsid = $class } | ConvertTo-Json
    Set-Content (Join-Path $installDir 'workstation.json') -Value $manifest -Encoding UTF8
    $null = New-Item -Path $classPath -Value 'Comunicação LAN' -Force
    $null = New-Item -Path ($classPath + '\InprocServer32') -Value (Join-Path $installDir 'lan_logon.dll') -Force
    $null = New-ItemProperty -Path ($classPath + '\InprocServer32') -Name ThreadingModel -PropertyType String -Value Apartment
    $null = New-Item -Path $providerPath -Value 'Comunicação LAN - Pix de teste' -Force
    Write-Host 'Entrada Windows instalada para a nova conta padrão:' $ClientUser
    Write-Host 'Na tela de entrada, selecione a conta e a opção Comunicação LAN. A senha aleatória não é exibida nem entregue ao cliente.'
    Write-Host 'O pagamento é simulado pelo painel. Nenhum Pix real é cobrado.'
} catch {
    Remove-Item $providerPath, $classPath -Recurse -Force -ErrorAction SilentlyContinue
    if ($installed) { & (Join-Path $PSScriptRoot 'uninstall-agent.ps1') -RemoveFiles }
    if ($created) { Remove-LocalUser -Name $ClientUser -ErrorAction SilentlyContinue }
    throw
} finally {
    Remove-Item Env:LAN_WINDOWS_GATE, Env:LAN_WINDOWS_CLIENT_USER, Env:LAN_WINDOWS_CLIENT_PASSWORD -ErrorAction SilentlyContinue
    [Array]::Clear($bytes, 0, $bytes.Length)
    $clear = $null
    $secure.Dispose()
}
