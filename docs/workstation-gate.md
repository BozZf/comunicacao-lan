# Bloqueio por sessão e compra na entrada do Windows

## Implementação

O modo de estação usa a tela de entrada do Windows como barreira de acesso.
O provedor adicional **Comunicação LAN** oferece pacote de tempo, QR de Pix de
teste e estado da cobrança. Quando o backend confirma o pagamento simulado,
o provedor solicita a entrada automática ao Windows. O serviço só libera uma
serialização de credenciais se houver tempo autorizado vigente.

Ao terminar o tempo, `WTSLogoffSession` encerra a sessão da conta dedicada e fecha seus programas.
Os avisos pedem para salvar o trabalho. Renovar antes do fim mantém a sessão
aberta; depois de expirar, a nova compra passa novamente pela autenticação do Windows. Fechar o acompanhamento gráfico não
desativa o serviço nem impede a expiração. A conta de manutenção não é alvo do
bloqueio, pois as sessões são identificadas pelo SID exato da conta cliente.

O agente neste modo roda como **LocalSystem**, necessário para consultar tokens
das sessões e encerrá-las. Ele não abre janelas privilegiadas no desktop do
cliente. O acompanhamento nativo continua como usuário padrão; a compra antes
da entrada acontece no LogonUI do Windows, por uma DLL Rust COM.

Um supervisor não interativo, em processo separado, mantém o prazo monotônico
recebido do agente e repete a revogação de acesso a cada 250 ms quando expira. Assim, o
reinício do serviço não depende de a janela do cliente estar aberta. Ao recuperar
o serviço, o supervisor anterior reconhece a nova instância assinada e encerra.
Uma parada administrativa sinaliza manutenção em arquivo protegido e encerra
a sessão cliente antes de encerrar os supervisores.

## Pix provisório

O adaptador `SimulatedPix` gera `LAN-DEMO:<id da cobrança>`, não um BR Code.
O QR é deliberadamente impossível de pagar por Pix. Preços são fictícios.
O painel oferece **Simular pagamento confirmado**, restrito ao atendente.
Isso substitui, somente para desenvolvimento, a confirmação do futuro provedor.

A interface e o provedor de entrada não podem confirmar o próprio pagamento.
O backend mantém a confirmação e o crédito na mesma transação idempotente.
A futura integração real deve substituir o adaptador, validar a cobrança no
provedor e remover/desabilitar a confirmação de simulação em produção.

## Credenciais e manutenção

O instalador cria uma **conta local padrão nova**, com senha aleatória longa que
não é exibida ao cliente, não muda a senha de contas existentes e não armazena
senha de administrador. O segredo da conta dedicada é protegido com DPAPI e ACL
em ProgramData. A chave privada do serviço também permanece nesse diretório.

Somente um chamador SYSTEM, identificado pelo token de impersonação do kernel,
pode solicitar serialização de entrada. O nome da conta, o tempo e a assinatura
não são aceitos como prova de identidade do chamador. O cliente padrão pode
consultar e comprar, mas não receber credenciais de entrada nem autorizar minutos.
As respostas são assinadas e vinculadas a um nonce novo por consulta.

A senha é recuperada em memória pelo serviço, protegida para transporte pelo
Windows com `CredProtectW` e serializada para o pacote Negotiate. Buffers locais
sensíveis são apagados ao liberar a memória. LogonUI recebe a serialização pelo
RPC local e a submete ao Windows; o aplicativo do cliente não participa disso.

O provedor **não é um filtro**. Nenhum provedor Microsoft é removido ou desativado,
e UAC/Ctrl+Alt+Del permanecem funcionando. A entrada administrativa normal deve
ser mantida e testada antes de ativar a conta cliente. Não cadastrar PIN/Hello ou
outro método de entrada conhecido pelo cliente nessa conta dedicada.

## Instalar em uma VM Windows Pro

Esta mudança integra autenticação e precisa de uma VM com snapshot e uma conta
administrativa de manutenção. O CI não substitui um teste interativo em Windows
Pro. Não instalar diretamente na máquina em uso da lan house antes desse teste.

Compile todos os componentes no repositório:

```powershell
cargo build -p lan-server -p lan-agent -p lan-station -p lan-logon
```

No PowerShell **administrativo**, configure e inicie o backend de simulação:

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
.\scripts\install-workstation.ps1 -ClientUser LanCliente
```

Se baixou o artefato `lan-windows-prototype`, os binários estão em `target/debug`;
não precisa compilar. Use o restante do roteiro preservando as pastas extraídas.

O instalador anterior `install-agent.ps1` sozinho continua sendo o modo de agente
visual, sem entrada Windows. Para bloqueio, use `install-workstation.ps1`.

Guarde os segredos do backend em local administrativo protegido. A demonstração
continua em loopback e depende do processo backend; troque de usuário sem encerrar
a sessão administrativa que o mantém aberto. Nada aqui processa Pix real.

## Roteiro gráfico de aceitação

1. Confirme que a entrada administrativa Microsoft continua acessível.
2. Troque de usuário para `LanCliente` e selecione a opção **Comunicação LAN**.
3. Escolha **Teste de 90 segundos** e clique em **Confirmar pacote e gerar Pix de teste**.
4. Confira o QR e a mensagem de simulação na própria tela de entrada.
5. Na sessão administrativa, abra http://127.0.0.1:8787/, use o token do atendente
   e confirme a cobrança de teste. O Windows deve tentar entrar automaticamente.
6. Na sessão cliente, abra um programa e observe o acompanhamento de tempo.
7. Renove pelo acompanhamento e confirme no painel; o prazo deve aumentar sem
   desconectar nem fechar o programa.
8. Deixe terminar. O Windows deve encerrar a sessão da conta cliente e exigir nova entrada;
   a opção Comunicação LAN deve permitir uma nova compra. Os programas devem ser fechados.
   Renovações antes do fim não devem encerrar a sessão.
9. Repita fechando o acompanhamento antes do fim, sem backend e após falha do
   serviço. O prazo autorizado não deve aumentar. Sem autorização recuperável,
   o comportamento esperado é manter a entrada bloqueada.

O Windows pode exigir selecionar a conta/opção de entrada e, em alguns contextos
de segurança, mostrar o botão **Entrar** antes de aceitar entrada automática.
O provedor solicita auto-logon pelos mecanismos suportados; não contorna essa
decisão do sistema. Se a autenticação falhar, não há repetição automática contínua
para a mesma revisão: o atendente precisa verificar a instalação/credencial.

Em algumas configurações a tela de bloqueio inicial precisa ser descartada antes
de exibir as opções de entrada. Não há código para interceptar a sequência segura
de teclas nem modificar a configuração global de entrada do Windows.

## Remover ou recuperar

Entre pela conta administrativa Microsoft. Remova apenas o componente do projeto:

```powershell
.\scripts\uninstall-workstation.ps1
# Para também excluir binários/identidade e a conta criada pelo instalador:
.\scripts\uninstall-workstation.ps1 -RemoveFiles -RemoveClientAccount
```

Por padrão, a conta dedicada é desativada e as identidades privadas são preservadas.
O instalador não apaga os dados persistidos do perfil. A expiração/parada do controle fecha programas da conta cliente. Revise os
dados antes de remover a conta. Para recuperação offline, as únicas chaves do
provedor são o CLSID `{E463AE81-2BA8-4FD3-948A-4BAF1809818E}` em `Classes/CLSID` e
na lista `Authentication/Credential Providers`; nenhum filtro do Windows é instalado.

## Limites de validação

Os testes automatizados verificam regras, assinaturas, acesso ao pipe, proteção
de credenciais, autenticação da serialização, ativação da DLL pelo COM registrado,
callbacks entre apartamentos COM e interfaces/bitmap. O teste do
serviço pode rodar em um Windows de CI sem desconectar o usuário do runner porque
o SID cliente é uma conta temporária distinta.

O ciclo real LogonUI → autenticação → desktop → expiração depende de sessão
interativa Windows Pro. Não deve ser declarado validado apenas porque compilou.
Também precisam ser exercitados suspensão, múltiplos monitores, jogos, drivers,
reinício de energia e limpeza de perfil entre clientes. O perfil ainda é compartilhado: arquivos e logins persistidos por aplicativos não são apagados automaticamente nesta versão. O backend ainda precisa
de reconciliação de uma cobrança paga cuja entrada falhe; não existe estorno real
porque nenhum pagamento real ocorre nesta versão.

Referências oficiais:

- https://learn.microsoft.com/en-us/windows/win32/secauthn/credential-providers-in-windows
- https://learn.microsoft.com/en-us/windows/win32/api/credentialprovider/nf-credentialprovider-icredentialprovider-getcredentialcount
- https://learn.microsoft.com/en-us/windows/win32/api/wtsapi32/nf-wtsapi32-wtslogoffsession
