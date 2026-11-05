# Documentação do wapit

> O wapit é uma WhatsApp Cloud API local para desenvolvimento: o Mailpit do WhatsApp. Aponte a URL base da Graph API do seu app para o wapit, mantenha o código de verdade e veja cada mensagem numa caixa de entrada local com cara de conversa. Ele não tem vínculo com a Meta nem com o WhatsApp.

## Instalação

Com o Homebrew, no macOS ou no Linux:

```bash
brew install patriciomartinns/tap/wapit
```

Ou rode o container. As duas últimas opções deixam o wapit entregar webhooks a apps na sua máquina, como `http://localhost:8000`, de dentro do container:

```bash
docker run --rm -p 127.0.0.1:8026:8026 -p 127.0.0.1:8027:8027 \
  --add-host host.docker.internal:host-gateway \
  -e WAPIT_LOOPBACK_HOST=host.docker.internal \
  ghcr.io/patriciomartinns/wapit
```

Ou compile a partir do código, com Rust 1.89 ou mais novo:

```bash
cargo install --locked --git https://github.com/patriciomartinns/wapit
```

No Windows, baixe `wapit-<versão>-x86_64-pc-windows-msvc.zip` da release mais recente, descompacte e rode `wapit.exe`. A mesma página tem os binários prontos para macOS e Linux e um arquivo `SHA256SUMS` para conferir cada um: https://github.com/patriciomartinns/wapit/releases/latest

## Primeiros passos

Rode `wapit` sem configuração nenhuma e ele serve um app padrão. A caixa de entrada fica em http://127.0.0.1:8026 e a Graph API em http://127.0.0.1:8027.

| Configuração | Valor |
| --- | --- |
| Phone number id | `100000000000001` |
| WhatsApp Business Account id | `200000000000001` |
| Access token | `wapit-local-token` |
| App secret | `wapit-local-secret` |
| URL do webhook | `http://localhost:8000/webhook` |
| Verify token do webhook | `wapit-verify-token` |

Use os mesmos valores no seu app, com `http://127.0.0.1:8027` como URL base da Graph API, e mande uma mensagem:

```bash
curl http://127.0.0.1:8027/v26.0/100000000000001/messages \
  -H 'Authorization: Bearer wapit-local-token' \
  -H 'Content-Type: application/json' \
  -d '{"messaging_product":"whatsapp","to":"15550000000","type":"text","text":{"body":"Hello"}}'
```

Uma mensagem livre para quem nunca falou com você é aceita e depois falha com o erro 131047, igual à Meta fora da janela de atendimento de 24 horas. Responda como o usuário pela caixa de entrada para abrir a janela.

O wapit aceita qualquer versão no caminho, ou nenhuma. Os exemplos usam a v26.0, a versão estável mais recente da Graph API, e os webhooks têm o formato da v24.0 em diante, sem o objeto `conversation` nos status.

## Configure seus apps

Copie `config/apps.example.json` para `config/apps.json` e descreva seus apps. Cada app é um número de WhatsApp:

| Campo | Significado |
| --- | --- |
| `id` | Nome curto, que também nomeia o arquivo de templates |
| `name` | Nome mostrado na caixa de entrada |
| `phone_number_id`, `display_phone_number`, `waba_id` | Os ids que o seu app usa nos caminhos da Graph API |
| `access_token` | Bearer token que o seu app envia |
| `app_secret` | Segredo usado para assinar os webhooks em `X-Hub-Signature-256` |
| `webhook_url` | Para onde o wapit envia mensagens e status |
| `verify_token` | Opcional. Permite rodar pela caixa de entrada a verificação `hub.challenge` da Meta contra o seu app |
| `auto_statuses` | Status disparados depois de um envio aceito. O padrão é `["sent"]` |
| `sync` | Opcional. De onde o `wapit sync` lê os templates reais |

O arquivo recarrega quando muda. Se uma edição quebrar o arquivo, o wapit mantém a última versão boa e mostra o erro na caixa de entrada.

## Templates

Os templates ficam em `config/templates/<id do app>.json`, no formato que o endpoint `message_templates` da Meta devolve. O seu app também pode criar, editar e apagar templates pela Graph API, e eles são aprovados na hora.

Copie os templates da sua conta real, só para leitura, com:

```bash
WAPIT_SYNC_TOKEN=... wapit sync <id do app>
```

Ele lê a Graph API v26.0, a menos que `sync.graph_version` do app diga outra versão. Os templates sincronizados mantêm os ids da Meta, então `POST /{template_id}` e `hsm_id` usam os mesmos ids da sua conta real. É o único comando que fala com a Meta, e só quando você roda.

## Variáveis de ambiente

| Variável | Padrão | Significado |
| --- | --- | --- |
| `WAPIT_CONFIG_DIR` | `./config` | Onde ficam `apps.json` e `templates/` |
| `WAPIT_DATA_DIR` | `./data` | Banco SQLite e arquivos de mídia |
| `WAPIT_HOST` | `127.0.0.1` | Endereço em que os dois servidores escutam |
| `WAPIT_API_PORT`, `WAPIT_UI_PORT` | `8027`, `8026` | Portas |
| `WAPIT_PUBLIC_API_URL` | `http://127.0.0.1:8027` | Base das URLs de download de mídia entregues aos apps |
| `WAPIT_PUBLIC_UI_URL` | `http://127.0.0.1:8026` | Para onde vai o navegador que abre a raiz da API |
| `WAPIT_LOOPBACK_HOST` | | Host que recebe os webhooks cujo host aponta para loopback. Use `host.docker.internal` num container |
| `WAPIT_CA_CERTS` | | Arquivo PEM com autoridades certificadoras extras para os webhooks, como uma CA local de desenvolvimento |
| `WAPIT_ALLOWED_HOSTS` | | Nomes de host extras que os servidores atendem, separados por vírgula |
| `WAPIT_LOG` | `info` | Filtro de log, por exemplo `wapit=debug` |

Um app rodando em outro container precisa chegar ao wapit por um nome que resolva lá, como `host.docker.internal:8027`. As URLs de mídia seguem `WAPIT_PUBLIC_API_URL`, então use esse mesmo endereço nela.

## Cobertura da Graph API

| Endpoint | Comportamento |
| --- | --- |
| `POST /{phone_number_id}/messages` | Texto, template, interativa (botões de resposta, lista, botão de URL, flow, produto único e múltiplos produtos, catálogo, pedido de permissão para ligar), mídia, reação, localização, contatos. `status: read`, com indicador de digitação opcional |
| `POST /{phone_number_id}/media` | Envia um arquivo e devolve o id. Só aceita os tipos que a Meta suporta, até 5 MB para imagens, 16 MB para áudio e vídeo, 100 MB para documentos e 500 KB para figurinhas. A Meta permite 500 KB para uma figurinha animada e 100 KB para uma estática; o wapit não distingue as duas, então toda figurinha fica com 500 KB |
| `GET /{media_id}`, `DELETE /{media_id}` | Metadados da mídia com URL de download, e exclusão |
| `GET /{waba_id}/message_templates` | Os templates locais, com filtros por `name`, `status`, `category` e `language`, e paginados com `limit`, `after` e `before` |
| `POST /{waba_id}/message_templates` | Cria um template, já aprovado |
| `POST /{template_id}` | Edita os componentes ou a categoria de um template. Só templates aprovados, rejeitados ou pausados podem ser editados, e um aprovado mantém a categoria |
| `DELETE /{waba_id}/message_templates?name=` | Apaga um template, um idioma dele com `hsm_id`, ou até 100 templates de uma vez com `hsm_ids`, tudo ou nada. Templates desativados não podem ser apagados |
| `GET /{waba_id}`, `GET /{phone_number_id}` | Campos da conta e do número, com resultado vazio de análise de preços |

Uma reação só recebe o status `sent`, como na Meta, e uma reação a uma mensagem que não está na conversa, que também é uma reação ou que tem mais de 30 dias é aceita e depois falha com o erro 131009.

## IDs de usuário por empresa

Envie com `recipient` preenchido com um ID de usuário por empresa (BSUID), como `US.13491208655302741918`, no lugar de `to`, e a resposta identifica o contato como `user_id` em vez de `wa_id`. Quando um envio traz os dois, vale o `to`, como na Meta. Os webhooks seguem o formato que a Meta usa desde abril de 2026: `contacts[].user_id`, `messages[].from_user_id` e `statuses[].recipient_user_id` vêm sempre, e `wa_id`, `from` e `recipient_id` só nas conversas com número de telefone. O wapit deriva um BSUID estável para cada número a partir do número e do id da conta do WhatsApp Business. Um envio cujo `recipient` é esse BSUID derivado vai para a conversa do número, dentro da janela de atendimento dela. Templates de autenticação de um toque, sem toque e de copiar código não podem ir para um BSUID (131062). Destinatários de grupo (`recipient_type: group`) não são suportados.

## Códigos de erro

Os envios são conferidos como a Meta confere. Um envio rejeitado devolve o erro da Meta ao seu app e fica na caixa de entrada, marcado, com o motivo em palavras simples.

| Código | Quando |
| --- | --- |
| `132000` | A quantidade de parâmetros no cabeçalho, no corpo ou nos botões de URL não bate com o template |
| `132001` | O nome ou o idioma do template não existe |
| `132005` | O texto passou do limite depois de preencher as variáveis |
| `132012` | O formato do cabeçalho não bate com o template |
| `132015` | O template está pausado |
| `132016` | O template está desativado |
| `132018` | Quebra de linha, tab ou mais de 4 espaços seguidos num parâmetro do corpo |
| `131008` | Falta o texto de um parâmetro |
| `131009` | Um valor fere as regras da Meta: um campo maior que o limite, ids ou títulos de botão repetidos, ou um índice ou tipo de botão que não bate com o template |
| `131047` | Mensagem livre fora da janela de atendimento de 24 horas |
| `131053` | Um upload passa do tamanho que o tipo permite, ou um arquivo enviado é mandado como um tipo de mensagem ao qual não pertence |
| `131062` | Um template de autenticação que precisa de número de telefone vai para um BSUID |

## Agindo como o usuário

Numa conversa você pode digitar uma mensagem, anexar um arquivo, tocar num botão de resposta, de resposta rápida ou de lista, ou escolher uma mensagem do seu app para marcar como entregue, lida, ouvida (só mensagens de voz) ou falha, com um código de erro real da Meta. Cada ação vai para o webhook do app no formato de payload da Meta, assinada com o app secret em `X-Hub-Signature-256`. Como na Meta, o objeto de preço vem com o status `sent` e com o primeiro entre `delivered` e `read`, nunca com os dois.

O inspetor mostra o payload que o seu app enviou, cada entrega de webhook com a resposta do app, e permite enviar um webhook de novo. Apps com `verify_token` também podem rodar a verificação da assinatura pela caixa de entrada.

O wapit entrega cada webhook uma vez só. Ao contrário da Meta, que reenvia uma entrega que falhou, ele não reenvia sozinho: se o seu app estiver fora do ar ou responder com erro, a entrega fica marcada como falha no inspetor e você envia de novo por lá.

## Lendo a inbox pelo terminal

`wapit messages` e `wapit show` leem o que um wapit em execução registrou. Assim você, um script ou um agente de código descobre por que um envio falhou sem abrir a inbox.

```sh
wapit messages --failed
wapit show <id da mensagem>
```

`wapit messages` lista as mensagens mais novas primeiro, de todas as conversas. `--failed` deixa só os envios rejeitados ou com falha e as mensagens com algum aviso, `--app` e `--contact` filtram por um app ou por um número ou BSUID, e `--limit` define quantas voltam (20 por padrão, até 500). `wapit show` mostra uma mensagem com a requisição que o seu app enviou, o erro e seus detalhes, os status e cada entrega de webhook com a resposta do seu app.

Os dois comandos leem a inbox em `http://127.0.0.1:8026` por padrão; aponte para outro endereço com `--inbox-url` ou `WAPIT_INBOX_URL`. Num container, rode-os dentro dele, por exemplo `docker exec <container> /wapit messages --failed`.

Use `--json` quando um programa for ler a saída. Ele imprime o que a API de leitura devolve: `GET /api/messages` (com os parâmetros `app`, `contact`, `failed=true` e `limit`) responde `{ "messages": [...] }`, e `GET /api/messages/{id}` responde uma mensagem. Cada mensagem tem `id`, `app_id`, `contact`, `direction`, `type`, `status`, `payload` (a requisição que o seu app enviou), `issues`, `error`, `context_id`, `created_at` em milissegundos, `statuses` e `webhooks`. Um envio que a Meta recusaria na hora tem `error` com `code`, `message` e `details`; um envio que falha depois, como o 131047 fora da janela de atendimento, tem `error` vazio e o motivo no status `failed` de `statuses`, em `error.code`, `error.message` e `error.error_data.details`. Cada webhook traz a URL, o código de status, a `response` do seu app e o `error` da entrega. A saída em texto pode mudar entre versões; esses campos do JSON só aumentam. `rendered` é o que a página da inbox desenha e pode mudar junto com ela.

Uma requisição recusada antes de o wapit saber quem é o destinatário, como token errado, phone number id desconhecido ou corpo sem `to`, recebe o erro e não é registrada, então só aparece na resposta HTTP do seu app.

## Segurança

O wapit não tem autenticação, de propósito: é uma ferramenta de desenvolvimento para a sua máquina.

- Ele escuta em `127.0.0.1`.
- Só atende requisições cujo `Host` é um nome local, então uma página da web não chega nele por DNS rebinding.
- A Graph API recusa escritas que trazem o `Origin` de um navegador, então uma página que você visita não consegue mandar mensagens por ele com o token padrão.

Não exponha as portas dele para fora da sua máquina. Relate vulnerabilidades em particular pelo GitHub: https://github.com/patriciomartinns/wapit/security
