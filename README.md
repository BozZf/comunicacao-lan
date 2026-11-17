# comunicacao-lan

Protótipo em Rust para compra de tempo em uma lan house Windows 10/11 Pro.

## O que esta versão faz

- Servidor HTTP local com catálogo de preços definido no servidor.
- Aplicativo nativo Rust/egui para PC-01: seleção de pacote, QR de demonstração,
  contagem regressiva, avisos e renovação.
- Painel web do atendente com credencial separada para confirmar o pagamento simulado.
- SQLite: cobranças, prazo da sessão, revisão e auditoria persistem após reiniciar o servidor.
- Confirmação e crédito na mesma transação, sem duplicação após reenvio ou reinício.
- Contagem monotônica local continua sem conexão e não é estendida por respostas repetidas.
- Pacote de 90 segundos para exercitar rapidamente compra, aviso e expiração.

**Não há Pix real nem bloqueio seguro do Windows nesta versão.** O QR contém
`LAN-DEMO:<id>` e não pode ser pago. A janela retorna à compra ao acabar o tempo,
mas pode ser fechada ou contornada. Não instalar como controle de uma lan house.
Não há serviço Windows, configuração de quiosque, limpeza do perfil ou instalador.

O servidor escuta somente em `127.0.0.1:8787`. A demonstração é de uma máquina;
servidor, painel e estação devem rodar no mesmo PC. Não expor por proxy/túnel.

## Executar no Windows Pro

Pré-requisitos para compilar: Rust estável com toolchain MSVC e Visual Studio
Build Tools com ferramentas C++/Windows SDK. Não requer execução como administrador.

Na pasta do repositório, em um terminal PowerShell:

```powershell
.\scripts\demo.ps1
```

O script compila os dois executáveis, gera duas credenciais aleatórias e inicia
servidor, navegador e estação. Cole no painel o token de atendente mostrado no
terminal. O token da estação não dá acesso às operações do atendente.

1. Na estação, selecione **Teste de 90 segundos**.
2. Veja o QR explicitamente marcado como demonstração.
3. Use Alt+Tab para abrir o painel e clique em **Simular pagamento confirmado**.
4. A estação recebe a autorização automaticamente e sai da tela cheia.
5. Acompanhe a contagem e o aviso de proximidade do fim.
6. Aos zero segundos, ela retorna automaticamente à tela cheia de compra.
7. Para testar renovação, compre outro pacote durante a sessão e confirme no painel.

Fechar a estação encerra a demonstração; o script também encerra o servidor.
O banco `lan-demo.sqlite3` permanece. Apague-o somente se quiser zerar todos os
dados de teste. Não há dinheiro real envolvido.

O workflow gera um artefato `lan-windows-prototype` quando o build Windows passa.
Depois de extrair mantendo a estrutura de pastas, execute:

```powershell
.\scripts\demo.ps1 -SkipBuild
```

Para rodar os executáveis separadamente, configure `LAN_ADMIN_TOKEN` e
`LAN_STATION_TOKEN` com valores distintos de pelo menos 32 caracteres, e defina
`LAN_DEMO_PAYMENTS=1` **somente para simulação**. Inicie `lan-server` e depois
`lan-station`, com a mesma credencial de estação. `LAN_DB` escolhe o arquivo SQLite.
O servidor pode iniciar sem a flag de simulação, mas não haverá nenhum meio de
confirmar pagamentos nesta versão. A rota de simulação recusa operações nesse caso.

## Organização

| Caminho | Função |
| --- | --- |
| `src/lib.rs` | Regras originais de sessão em memória e seus testes |
| `src/protocol.rs` | Contratos compartilhados e catálogo fictício |
| `src/lease.rs` | Contagem monotônica usada pela estação |
| `crates/server` | API Axum, SQLite transacional e painel web |
| `crates/station` | Aplicativo nativo e consultas autenticadas ao servidor |
| `scripts/demo.ps1` | Inicialização local no Windows |
| `scripts/smoke.ps1` | Teste HTTP real com reinício do servidor |

O armazenamento transacional evolui as regras da primeira biblioteca e tem testes
próprios. O servidor é a autoridade do crédito; a interface nunca envia preço,
duração ou identificador de máquina para autorizar a sessão.

## Verificação

```powershell
cargo test -p comunicacao-lan -p lan-server --all-targets
cargo build -p lan-server -p lan-station
.\scripts\smoke.ps1
```

Os testes cobrem domínio, avisos, contagem offline, latência, respostas antigas,
renovação, rollback de transação, expiração da cobrança, autenticação, origem,
Host, persistência e confirmação repetida depois de reiniciar.
O CI executa testes de backend em Windows e Linux, compila o aplicativo no Windows
 e faz o teste HTTP. Compilar não comprova o comportamento gráfico: o roteiro
manual acima precisa ser executado em uma VM Windows Pro com sessão interativa.

## Limites de segurança e recuperação

- Tokens distintos, sem armazenamento no navegador; credencial do painel só em memória.
- API verifica Bearer, Host e Origin. Não habilita CORS; corpo limitado a 1 KiB.
- Catálogo é controlado pelo servidor. Pedidos com preço/duração extras são rejeitados.
- Uma cobrança pendente por vez; reenvio do mesmo pacote reutiliza a cobrança.
- Cobrança pendente expira em 10 minutos; confirmação fora do prazo é recusada.
- Um recibo de demonstração é creditado uma única vez, com auditoria na mesma transação.
- Durante execução, o servidor usa relógio monotônico. Ao reiniciar, recusa horário
  anterior ao último instante persistido; correção do relógio exige intervenção.
- A estação usa prazo monotônico, desconta latência e nunca prolonga a mesma revisão.
  Ela inicia sem autorização e só recupera a sessão após consultar o servidor.
- Se iniciar sem rede, não recupera crédito offline; precisa reconectar.
- O tempo começa na confirmação simulada, mesmo se a estação estiver desligada.
  A reconciliação de pagamento sem liberação efetiva ainda não está implementada.
- Tokens no ambiente e banco local não são proteção contra outro processo do mesmo
  usuário. O serviço protegido e o provisionamento seguro ainda precisam ser feitos.
- Mudança de horário entre reinícios, suspensão, falha do aplicativo e tela
  minimizada precisam de validação no agente definitivo. Este protótipo não
  fornece garantia de revogação de acesso nesses cenários.

## Próxima etapa: proteção real no Windows Pro

Separar o serviço Windows da interface e proteger a comunicação local com ACLs.
O usuário de cliente será padrão, com administrador de manutenção separado.
Não usar senha de administrador, desativar UAC ou interceptar Ctrl+Alt+Del.

O Pro oferece Assigned Access; avaliar a configuração compatível com a build e
os programas/jogos instalados. Shell Launcher não está disponível no Pro.
Uma lista estática de apps permitidos não resolve sozinha a transição entre compra
e sessão paga. `LockWorkStation` também não resolve liberação automática, pois
o desbloqueio padrão exige login. O mecanismo de acesso ainda não foi escolhido
nem validado. Antes do piloto, testar tentativas de fechar a interface, atalhos,
reinícios, encerramento do serviço, perda de rede, renovação e isolamento de dados.

Depois: múltiplas máquinas com credenciais revogáveis, TLS, autorização com revisão
protegida contra replay, Pix real com consulta autenticada ao provedor,
reconciliação/estorno e limpeza de sessão entre clientes.

Referências oficiais:

- https://learn.microsoft.com/en-us/windows/win32/services/interactive-services
- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-lockworkstation
- https://learn.microsoft.com/en-us/windows/configuration/assigned-access/
- https://learn.microsoft.com/en-us/windows/configuration/shell-launcher/
