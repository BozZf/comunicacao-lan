# Registra somente o provedor adicional do projeto, sem filtros de entrada.
$ErrorActionPreference = 'Stop'
if (-not [Environment]::Is64BitProcess) { throw 'Use PowerShell de 64 bits.' }
$class = '{E463AE81-2BA8-4FD3-948A-4BAF1809818E}'
$classPath = 'HKLM:\SOFTWARE\Classes\CLSID\' + $class
$providerPath = 'HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\' + $class
if ((Test-Path $classPath) -or (Test-Path $providerPath)) { throw 'O provedor já está registrado.' }
$root = Join-Path ([Environment]::GetFolderPath('ProgramFiles')) 'ComunicacaoLanAgent'
$dll = Join-Path $root 'lan_logon.dll'
if (-not (Test-Path $dll) -or -not (Test-Path (Join-Path $root 'agent-public.json'))) { throw 'Instalação do agente/DLL ausente.' }
try {
    $null = New-Item -Path $classPath -Value 'Comunicação LAN' -Force
    $null = New-Item -Path ($classPath + '\InprocServer32') -Value $dll -Force
    $null = New-ItemProperty -Path ($classPath + '\InprocServer32') -Name ThreadingModel -PropertyType String -Value Apartment
    $null = New-Item -Path $providerPath -Value 'Comunicação LAN - Pix de teste' -Force
} catch {
    Remove-Item $providerPath, $classPath -Recurse -Force -ErrorAction SilentlyContinue
    throw
}
