# wapit documentation

> wapit is a local WhatsApp Cloud API for development: Mailpit, but for WhatsApp. Point your app's Graph API base URL at wapit, keep your real code paths, and see each message in a local inbox that looks like a chat. It is not affiliated with Meta or WhatsApp.

## Install

With Homebrew on macOS or Linux:

```bash
brew install patriciomartinns/tap/wapit
```

Or run the container. The last two options let wapit deliver webhooks to apps on your machine, such as `http://localhost:8000`, from inside the container:

```bash
docker run --rm -p 127.0.0.1:8026:8026 -p 127.0.0.1:8027:8027 \
  --add-host host.docker.internal:host-gateway \
  -e WAPIT_LOOPBACK_HOST=host.docker.internal \
  ghcr.io/patriciomartinns/wapit
```

Or build from source with Rust 1.89 or newer:

```bash
cargo install --locked --git https://github.com/patriciomartinns/wapit
```

On Windows, download `wapit-<version>-x86_64-pc-windows-msvc.zip` from the latest release, unzip it and run `wapit.exe`. The same page has prebuilt binaries for macOS and Linux, and a `SHA256SUMS` file to check them: https://github.com/patriciomartinns/wapit/releases/latest

## Quick start

Run `wapit` with no configuration and it serves one default app. The inbox is on http://127.0.0.1:8026 and the Graph API on http://127.0.0.1:8027.

| Setting | Value |
| --- | --- |
| Phone number id | `100000000000001` |
| WhatsApp Business Account id | `200000000000001` |
| Access token | `wapit-local-token` |
| App secret | `wapit-local-secret` |
| Webhook URL | `http://localhost:8000/webhook` |
| Webhook verify token | `wapit-verify-token` |

Set the same values in your app, with `http://127.0.0.1:8027` as the Graph API base URL, and send a message:

```bash
curl http://127.0.0.1:8027/v23.0/100000000000001/messages \
  -H 'Authorization: Bearer wapit-local-token' \
  -H 'Content-Type: application/json' \
  -d '{"messaging_product":"whatsapp","to":"15550000000","type":"text","text":{"body":"Hello"}}'
```

A free-form message to someone who never wrote to you is accepted and then fails with error 131047, exactly as Meta does outside the 24-hour service window. Reply as the user from the inbox to open the window.

## Configure your apps

Copy `config/apps.example.json` to `config/apps.json` to describe your own apps. Each app is one WhatsApp phone number:

| Field | Meaning |
| --- | --- |
| `id` | Short name, also the name of its templates file |
| `name` | Name shown in the inbox |
| `phone_number_id`, `display_phone_number`, `waba_id` | The ids your app uses in Graph API paths |
| `access_token` | Bearer token your app sends |
| `app_secret` | Secret used to sign webhooks in `X-Hub-Signature-256` |
| `webhook_url` | Where wapit posts messages and statuses |
| `verify_token` | Optional. Lets the inbox run Meta's `hub.challenge` verification against your app |
| `auto_statuses` | Statuses fired after an accepted send. Defaults to `["sent"]` |
| `sync` | Optional. Where `wapit sync` reads real templates from |

The file reloads when it changes. If an edit breaks it, wapit keeps the last good version and shows the error in the inbox.

## Templates

Templates live in `config/templates/<app id>.json`, in the format Meta's `message_templates` endpoint returns. Your app can also create, edit and delete them through the Graph API, and they are approved right away.

Copy them from your real account, read-only, with:

```bash
WAPIT_SYNC_TOKEN=... wapit sync <app id>
```

This is the only command that talks to Meta, and only when you run it.

## Environment

| Variable | Default | Meaning |
| --- | --- | --- |
| `WAPIT_CONFIG_DIR` | `./config` | Where `apps.json` and `templates/` live |
| `WAPIT_DATA_DIR` | `./data` | SQLite database and media files |
| `WAPIT_HOST` | `127.0.0.1` | Address both servers bind to |
| `WAPIT_API_PORT`, `WAPIT_UI_PORT` | `8027`, `8026` | Ports |
| `WAPIT_PUBLIC_API_URL` | `http://127.0.0.1:8027` | Base of the media download URLs given to apps |
| `WAPIT_PUBLIC_UI_URL` | `http://127.0.0.1:8026` | Where a browser opening the API root is sent |
| `WAPIT_LOOPBACK_HOST` | | Host that receives webhooks whose host resolves to loopback. Set it to `host.docker.internal` in a container |
| `WAPIT_CA_CERTS` | | PEM file of extra certificate authorities trusted for webhooks, such as a local development CA |
| `WAPIT_ALLOWED_HOSTS` | | Extra host names the servers answer to, comma separated |
| `WAPIT_LOG` | `info` | Log filter, e.g. `wapit=debug` |

An app running in another container must reach wapit by a name that resolves there, such as `host.docker.internal:8027`. Media URLs follow `WAPIT_PUBLIC_API_URL`, so set it to that same address.

## Graph API coverage

| Endpoint | Behavior |
| --- | --- |
| `POST /{phone_number_id}/messages` | Text, template, interactive (reply buttons, list, call to action URL), media, reaction, location, contacts. `status: read`, with an optional typing indicator |
| `POST /{phone_number_id}/media` | Uploads a file and returns its id. Only Meta's supported types are accepted, up to 5 MB for images, 16 MB for audio and video, 100 MB for documents and 500 KB for stickers |
| `GET /{media_id}`, `DELETE /{media_id}` | Media metadata with a download URL, and deletion |
| `GET /{waba_id}/message_templates` | The local templates, filtered by `name`, `status`, `category` and `language`, and paged with `limit`, `after` and `before` |
| `POST /{waba_id}/message_templates` | Creates a template, approved right away |
| `POST /{template_id}` | Edits a template's components or category. Only approved, rejected or paused templates can be edited, and an approved one keeps its category |
| `DELETE /{waba_id}/message_templates?name=` | Deletes a template, one language of it with `hsm_id`, or up to 100 templates at once with `hsm_ids`, all or nothing. Disabled templates cannot be deleted |
| `GET /{waba_id}`, `GET /{phone_number_id}` | Account and phone number fields, with an empty pricing analytics result |

A reaction gets only the `sent` status, as on Meta, and a reaction to a message that is not in the chat, is itself a reaction or is more than 30 days old is accepted and then fails with error 131009.

## Error codes

Sends are checked like Meta checks them. A rejected send returns Meta's error to your app and stays in the inbox, flagged, with the reason in plain words.

| Code | When |
| --- | --- |
| `132000` | Parameter count in the header, body or URL buttons does not match the template |
| `132001` | Template name or language does not exist |
| `132005` | Text too long after the variables are filled in |
| `132012` | Header format does not match the template |
| `132015` | Template is paused |
| `132016` | Template is disabled |
| `132018` | Line breaks, tabs or more than 4 spaces in a row in a body parameter |
| `131008` | A parameter is missing its text |
| `131009` | Button index or type does not match the template |
| `131047` | Free-form message outside the 24-hour service window |
| `131053` | An upload is larger than its type allows, or an uploaded file is sent as a message type it does not belong to |

## Acting as the user

In a conversation you can type a message, attach a file, tap a reply, quick reply or list button, or select one of your app's messages to mark it delivered, read, played (voice messages only) or failed with a real Meta error code. Every action is posted to the app's webhook in Meta's payload format, signed with the app secret in `X-Hub-Signature-256`. As on Meta, the pricing object comes with the `sent` status and with the first of `delivered` or `read`, never with both.

The inspector shows the payload your app sent, each webhook delivery with your app's response, and lets you send a webhook again. Apps with a `verify_token` can also run the subscription handshake from the inbox.

wapit delivers each webhook once. Unlike Meta, which retries a failed delivery, it does not retry on its own: if your app is down or answers with an error, the delivery is marked failed in the inspector and you send it again from there.

## Security

wapit has no authentication by design: it is a development tool for your own machine.

- It binds to `127.0.0.1`.
- It only answers requests whose `Host` is a local name, so a web page cannot reach it through DNS rebinding.
- Its Graph API refuses writes that carry a browser's `Origin`, so a page you visit cannot send messages through it with the default token.

Do not publish its ports beyond your machine. Report vulnerabilities privately through GitHub: https://github.com/patriciomartinns/wapit/security
