# comunicacao-lan

Controle de tempo de uma lan house Windows 10/11 Pro x64, em Rust, com Pix de teste.

## Fluxo implementado

1. Na conta dedicada, a opção **Comunicação LAN** da tela de entrada do Windows
   permite escolher um pacote e gerar o QR de teste.
2. O backend cria a cobrança. No painel, o atendente simula a confirmação do Pix.
3. O serviço recebe uma autorização assinada. O provedor solicita entrada automática
   ao Windows, sem entregar senha ao cliente nem à interface de acompanhamento.
4. O acompanhamento nativo mostra tempo e renovação. O serviço também envia avisos.
5. Quando o tempo acaba, o serviço e um supervisor independente encerram a sessão
   Windows da conta cliente. Os programas são fechados: os avisos pedem para salvar
   o trabalho. Renovar antes do fim mantém a sessão aberta.

**Pix é simulado e não movimenta dinheiro.** O QR usa `LAN-DEMO:<id>`, não BR Code.
A confirmação manual no painel substitui apenas o futuro provedor de pagamentos.
A interface do cliente não pode confirmar sua própria cobrança.

A integração com LogonUI e o bloqueio por `WTSLogoffSession` estão implementados;
**o ciclo gráfico completo ainda precisa ser validado em VM Windows Pro**.
Compilação, testes COM e testes de serviço não substituem essa validação.

## Escolher o modo

| Modo | Como executar | Controle do Windows |
| --- | --- | --- |
| Demonstração direta | `scripts/demo.ps1` | Somente janela de demonstração, sem bloqueio |
| Agente visual | `scripts/install-agent.ps1` | Serviço LocalService e IPC protegido, sem revogação de acesso |
| Entrada Windows | `scripts/install-workstation.ps1` | Conta nova, provedor LogonUI e encerramento por tempo via SYSTEM |

Para o fluxo com bloqueio, siga [workstation-gate.md](docs/workstation-gate.md).
O instalador cria uma conta padrão nova com senha aleatória e registra um provedor
adicional. Não altera provedores Microsoft, UAC ou Ctrl+Alt+Del. A entrada normal
administrativa continua disponível. Instale primeiro em uma VM com snapshot.

O backend de demonstração ainda escuta somente em `127.0.0.1:8787`. Servidor,
painel e estação precisam estar no mesmo PC; não expor por proxy/túnel.
Ainda não há múltiplas estações nem Pix real.

## Demonstração visual rápida, sem instalar serviço

Com Rust estável MSVC e ferramentas de compilação C++/Windows SDK instaladas:

```powershell
.\scripts\demo.ps1
```

Escolha o teste de 90 segundos. Use o token administrativo mostrado no terminal
para entrar no painel e simular a confirmação. Esse modo é útil para ver a compra
e renovação, mas pode ser fechado e não representa o bloqueio do sistema.

O workflow disponibiliza o artefato `lan-windows-prototype`, com executáveis,
DLL de entrada, scripts e documentação. Depois de extrair preservando as pastas,
a demonstração visual também pode ser iniciada sem compilar:

```powershell
.\scripts\demo.ps1 -SkipBuild
```

Instalar a entrada Windows é uma operação administrativa separada, descrita no
roteiro de VM. Não confundir o script da demonstração com o instalador do bloqueio.

## Componentes

| Caminho | Responsabilidade |
| --- | --- |
| `src/lib.rs` | Regras originais de sessão e testes |
| `src/lease.rs` | Contagem monotônica, latência e revisão sem extensão por replay |
| `src/protocol.rs` | Catálogo fictício e contratos |
| `crates/server` | API Axum, SQLite transacional, adaptador de Pix simulado e painel |
| `crates/agent` | Serviço Windows, supervisor, controle de sessão e credencial DPAPI |
| `crates/transport` | Assinaturas Ed25519, nonce, limites e named pipe |
| `crates/logon` | DLL Rust COM: compra e serialização na entrada do Windows |
| `crates/station` | Acompanhamento gráfico, avisos e renovação |

O servidor controla preço/duração. Cobrança e crédito são confirmados na mesma
transação, com identificação única e auditoria. Repetir a confirmação, inclusive
após reinício, não concede minutos de novo. O SQLite é `lan-demo.sqlite3` por
padrão; `LAN_DB` permite escolher o caminho.

## Testar

```powershell
cargo test -p comunicacao-lan -p lan-server -p lan-transport -p lan-agent -p lan-logon --all-targets
cargo build -p lan-server -p lan-agent -p lan-station -p lan-logon
.\scripts\smoke.ps1
```

Somente em CI/VM descartável, como administrador:

```powershell
.\scripts\service-smoke.ps1
.\scripts\service-smoke.ps1 -WindowsGate
```

Os testes de serviço criam contas temporárias, verificam ACLs, compra, assinatura,
contagem offline, reinício após falha e recusa de serialização de entrada ao
cliente. No modo WindowsGate, o diagnóstico SYSTEM decodifica a serialização e
verifica a credencial pela autenticação do Windows, sem iniciar um desktop.
Testes COM verificam campos, seleção, bitmap e ausência de auto-logon sem crédito.
O roteiro gráfico de aceitação permanece necessário em Windows Pro.

## Segurança e limites

- Contas cliente e atendente têm credenciais distintas. Operações locais não
  concedem tempo nem confirmam pagamentos.
- Backend e agente assinam respostas com chaves distintas e desafio por consulta.
  Não há fallback não assinado no modo agente.
- Pipe restrito por SID, clientes remotos recusados, mensagens limitadas e timeout.
- Senha da conta dedicada protegida por DPAPI + ACL. Somente SYSTEM pode solicitar
  serialização para entrada, e apenas durante crédito autorizado.
- Ao expirar, encerra sessões ativas ou desconectadas do SID cliente; não escolhe
  sessão por nome nem interfere na conta administrativa.
- Um supervisor separado mantém o limite durante falha do serviço. Se o agente
  reiniciar sem backend, começa sem crédito até obter autorização assinada.
- O painel mostra autorização de tempo, não prova de que o desktop já abriu.
- O tempo começa na confirmação simulada. Reconciliação de pagamento cuja entrada
  falhe ainda não está implementada; não há estorno real porque não há dinheiro real.
- Não há limpeza automática de perfil: arquivos e logins salvos por aplicativos
  ainda podem persistir entre clientes. Isso precisa ser resolvido antes do piloto.
- Suspensão, energia, drivers, jogos, múltiplos monitores e a integração gráfica
  precisam ser exercitados na VM. Windows pode exigir selecionar a conta/opção de
  entrada ou confirmar **Entrar** em determinados contextos de segurança.

A integração real de Pix deverá substituir o adaptador `SimulatedPix`, consultar
cobrança autenticada no provedor, conferir recebedor/moeda/valor e retirar a rota
de simulação. Nesta versão, a confirmação só funciona com `LAN_DEMO_PAYMENTS=1`.

## Documentação

- [Entrada Windows e roteiro de aceitação](docs/workstation-gate.md)
- [Agente visual LocalService](docs/windows-agent.md)
