# Agente Windows experimental

Esta etapa separa o controle de autorização da janela de compra. O agente é um
serviço Windows real, executado como LocalService, com SID próprio e recuperação
pelo SCM. **Ainda não revoga acesso ao desktop, não encerra jogos e não substitui
um bloqueio do Windows.** Fechar a interface continua possível. O serviço mantém
o prazo da sessão, mas não obriga a interface a permanecer aberta.

## O que mudou

- O serviço guarda a credencial da estação e consulta o backend.
- Backend e serviço assinam respostas com identidades Ed25519 distintas.
- Cada consulta contém um nonce aleatório. Respostas adulteradas, de outra chave
  ou de uma consulta anterior são rejeitadas; não existe fallback para HTTP sem assinatura.
- A interface instalada conhece somente a chave pública do serviço.
- RPC local por named pipe, somente para a conta local de cliente provisionada,
  SYSTEM e o SID do serviço. Clientes remotos são recusados.
- A conta cliente pode ler/escrever no pipe, mas não criar instâncias de servidor.
  A próxima instância é reservada antes de liberar a anterior. Um ocupante anterior
  faz o agente falhar ao iniciar, em vez de assumir um canal de origem desconhecida.
- Mensagens têm tamanho máximo, timeout e limite de conexões concorrentes.
- Cliente utiliza nível de impersonação Identification, sem conceder ao pipe a
  possibilidade de executar operações usando a identidade do usuário.
- Operações disponíveis: estado, catálogo e compra. Não há comando local para
  conceder minutos, confirmar pagamento, mudar revisão ou fornecer chave privada.
- Configuração privada em ProgramData: acesso somente a SYSTEM, administradores
  e leitura pelo SID do serviço. Não conceder acesso a todos os serviços LocalService.
- Binários e chave pública em Program Files: usuários leem/executam, mas não alteram.
- Conta cliente não pode parar nem configurar o serviço; administradores mantêm
  controle de manutenção. Falha abrupta aciona recuperação configurada no SCM.

Essas propriedades dependem do instalador e dos diretórios protegidos. Executar
os binários a partir de uma pasta gravável pelo cliente não estabelece essa proteção.
Administrador/SYSTEM e comprometimento do backend estão fora desse limite de confiança.
O backend permanece uma demonstração em loopback de uma máquina, com Pix simulado.

## Preparar a VM

Use uma VM Windows 10/11 Pro com uma conta administrativa de manutenção e outra
conta **local padrão**, fora de Administradores, chamada `cliente` neste exemplo.
Não execute a instalação em uma máquina de produção. O script altera serviço,
ACLs e, opcionalmente, uma tarefa de inicialização da interface.

Compile em um terminal PowerShell no repositório:

```powershell
cargo build -p lan-server -p lan-station -p lan-agent
```

Em um terminal **administrativo** na mesma pasta, configure o backend:

```powershell
$env:LAN_ADMIN_TOKEN = [guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N')
$env:LAN_STATION_TOKEN = [guid]::NewGuid().ToString('N') + [guid]::NewGuid().ToString('N')
$env:LAN_DEMO_PAYMENTS = '1'
$keys = & .\target\debug\lan-server.exe --generate-agent-key
foreach ($line in $keys) {
    $pair = $line.Split('=', 2)
    [Environment]::SetEnvironmentVariable($pair[0], $pair[1], 'Process')
}
$backend = Start-Process .\target\debug\lan-server.exe -PassThru
.\scripts\install-agent.ps1 -ClientUser cliente -StartInterfaceAtLogon
```

O instalador recusa contas locais administrativas e instalações existentes; não
sobrescreve identidades. Preserve a chave de assinatura do backend em um local
administrativo protegido se quiser recuperar esta instalação após fechar o
terminal. Gerar outra chave faz o agente rejeitar o backend até reprovisionamento.
Não coloque chaves ou tokens no Git. A chave privada do serviço é gerada localmente
e não sai do arquivo privado; a chave privada do backend nunca é passada ao agente.

Troque para a conta `cliente` sem encerrar a sessão administrativa que mantém
este backend de demonstração aberto. A tarefa opcional inicia a interface no login.
Também é possível executar manualmente nessa conta:

```powershell
& "$env:ProgramFiles\ComunicacaoLanAgent\lan-station.exe" --agent
```

O painel continua em http://127.0.0.1:8787/. Use-o na sessão administrativa com o
token do atendente; não entregue essa credencial à conta cliente.
Selecione um pacote na estação e confirme o pagamento simulado pelo painel.

Para inspecionar somente o estado assinado, na conta cliente:

```powershell
& "$env:ProgramFiles\ComunicacaoLanAgent\lan-agent.exe" --probe
```

Esse comando não fornece segredos. A mesma operação sob outra conta padrão deve
ser recusada. A interface não precisa de `LAN_STATION_TOKEN` no modo agente.
O modo antigo de demonstração direta exige `lan-station --demo` explicitamente.

## Recuperação e teste

O serviço mantém prazo monotônico e o backend persiste sessão/revisão. Reiniciar a
interface recupera o prazo atual do serviço, sem conceder tempo novo. Sem backend,
o serviço continua descontando o prazo recebido, mas não aceita novas compras.
Se o serviço reiniciar sem backend, começa sem crédito até recuperar uma autorização
assinada. Não há persistência offline de autorização no agente nesta versão.

O teste automático `scripts/service-smoke.ps1`, restrito a CI/VM descartável,
cria duas contas locais temporárias, instala o serviço e verifica:

1. Comunicação assinada sob conta padrão e recusa de outra conta no pipe.
2. Recusa de leitura da configuração privada e de parada do serviço pelo cliente.
3. Compra pelo pipe e recebimento do pagamento confirmado no backend.
4. Contagem sem backend e manutenção da sessão entre processos de interface.
5. Reinício automático após matar o agente e recuperação sem duplicar crédito.

O teste remove serviço, arquivos e contas temporárias ao terminar. Não usa uma
sessão gráfica nem atesta segurança do bloqueio do desktop. Testes de assinatura,
nonce, revisão e contagem também rodam em Windows e Linux.

Para remover a instalação experimental, em terminal administrativo:

```powershell
.\scripts\uninstall-agent.ps1
# Para remover também binários e identidade privada:
.\scripts\uninstall-agent.ps1 -RemoveFiles
```

## Critério antes de usar numa lan house

A próxima implementação precisa revogar acesso independentemente da janela.
Validar mecanismo de restrição no Pro, compatibilidade com jogos, atalhos, troca
de usuário, suspensão, múltiplos monitores, crash, recuperação, liberação automática
e isolamento do perfil entre clientes. Não usar `LockWorkStation` como se permitisse
desbloqueio automático; não desativar UAC nem tentar interceptar Ctrl+Alt+Del.

Os testes de serviço são uma base de comunicação e ciclo de vida, não evidência
de que a estação já impede uso sem pagamento. Não há Pix real nesta entrega.
