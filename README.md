# comunicacao-lan

Projeto em Rust para compra de tempo por Pix em uma lan house Windows 10/11.

## Estado atual

Primeira base: biblioteca de regras de sessão, exemplo de terminal e testes.
Ainda NÃO é um aplicativo instalável: não há servidor HTTP, integração Pix,
QR code, painel, serviço Windows ou bloqueio real. Nada altera o Windows.
Os dados ficam em memória e são perdidos ao encerrar o processo.

```powershell
cargo test --all-targets
cargo run --example session_demo
```

O exemplo avança um relógio simulado de uma hora sem esperar uma hora real.
O código não usa dependências externas. O workflow verifica Linux e Windows.

## Regras implementadas

- Criar uma cobrança não libera a máquina.
- Uma confirmação interna deve corresponder à cobrança, máquina e valor esperado.
- Repetir a mesma confirmação não adiciona tempo novamente na mesma instância.
- Renovação soma ao prazo existente; depois da expiração começa no momento da confirmação.
- Avisos em 10, 5 e 1 minuto; um tick atrasado mostra apenas o aviso mais urgente.
- Expiração emite um evento de bloqueio, inclusive sem acesso à rede.
- Relógio regressivo e soma de tempo com overflow são rejeitados.

`confirm_payment` NÃO é uma API pública e NÃO comprova que houve pagamento.
É uma operação de domínio para ser chamada pelo futuro servidor somente após
validar o provedor. Um processo local capaz de chamar essa função pode simular
pagamentos; esta biblioteca isolada não constitui uma fronteira de segurança.

## Arquitetura prevista

| Parte | Responsabilidade |
| --- | --- |
| Servidor Rust | Catálogo, cobranças, confirmação Pix, sessões e auditoria |
| Serviço Windows Rust | Verificar autorizações, controlar prazo e acionar a integração de acesso |
| Interface da estação | Seleção de pacote, QR code, avisos e renovação |
| Painel do atendente | Estado das máquinas, manutenção e operações excepcionais autenticadas |

O serviço e a interface serão processos separados. O serviço não deve abrir
janelas diretamente. A comunicação local deverá ter ACLs, verificar identidade
e limitar operações; a interface nunca autoriza tempo nem recebe segredos Pix.

## Plano para segurança e confiabilidade

1. Adaptador Pix: validar assinatura/autenticidade de notificações conforme o
   provedor e consultar a cobrança autenticada; conferir recebedor, moeda BRL,
   valor, identificador e estado aprovado. Não confiar em prints ou no navegador.
2. Persistência transacional: cobrança e crédito na mesma transação, identificador
   de cobrança único globalmente, confirmação idempotente mesmo após reinício.
3. Provisionamento: credencial revogável por máquina, TLS, autorização com máquina,
   sessão, revisão e prazo; rejeitar replay de autorizações antigas.
4. Agente: prazo monotônico durante execução e prazo absoluto autenticado para
   recuperação. Não confiar no relógio editável do cliente; se não puder recuperar
   uma autorização válida, manter acesso restrito e pedir intervenção.
5. Sem rede: respeitar o prazo já autorizado; novas compras e renovações só valem
   quando confirmadas. Reconectar não prolonga o tempo.
6. Usuário padrão, serviço com permissões mínimas, binários/configuração protegidos
   e recuperação do serviço; administrador separado para manutenção.
7. O painel deverá distinguir autorização emitida de liberação efetivamente
   aplicada. Se uma máquina não puder iniciar uma sessão paga, reconciliar a
   cobrança e oferecer atendimento/estorno sem consumir silenciosamente o tempo.
8. Validar recuperação, perda de rede, queda de energia e manipulação de horário
   antes de qualquer piloto. Os testes atuais cobrem apenas o domínio em memória.

## Integração Windows: decisão ainda aberta

Não usar `LockWorkStation` como mecanismo completo de compra: o desbloqueio
padrão exige login, e a função precisa de um processo no desktop interativo.
Uma janela em tela cheia também não constitui proteção suficiente.

É preciso confirmar a edição (Home, Pro, Enterprise/Education) e validar a
estratégia de acesso em uma VM. Shell Launcher depende de edições específicas
e, sozinho, não impede acesso a outros componentes. Não armazenar senha de
administrador para automatizar login, nem desativar UAC ou Ctrl+Alt+Del.

Ao expirar, a integração deve revogar o acesso e retornar à compra, com política
explícita para aplicativos abertos e dados do usuário. A conta do próximo cliente
não deve herdar logins e arquivos do anterior. Isso ainda será implementado.

Referências oficiais:

- https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-lockworkstation
- https://learn.microsoft.com/en-us/windows/win32/services/interactive-services
- https://learn.microsoft.com/en-us/windows/configuration/shell-launcher/

## Próximas entregas

1. Servidor com banco e adaptador de pagamento simulado isolado de produção.
2. Interface de compra e painel com autenticação.
3. Serviço e integração Windows testados na edição escolhida.
4. Provedor Pix real, reconciliação e piloto em uma máquina.
