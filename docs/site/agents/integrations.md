---
title: Integrations
description: Give agents tools from Notion, Linear, GitHub and other MCP servers, without handing them a credential.
---

An **integration** is another MCP server — Notion, Linear, GitHub, or any
service with one — that you connect **once**, in the app. Every agent session
except a shell can then use it, within the limits you set.

Storm keeps the credential on your server, encrypted, and calls the service
on the agent's behalf. **An agent never receives the credential**: its host
is told only which integrations exist.

## Connecting one

In **Settings › Integrations › Add integration**, give it a name and the
service's **MCP server address**, then choose how Storm signs in:

- **Sign in** — the service's own sign-in (OAuth), in your browser. Storm
  finds how the service signs in, and relays only the result. Sign-in works
  from the macOS and Android apps; in the web app, use a token instead.
- **A token** — an API or personal access token, sent as a header. For
  GitHub, use a fine-grained token.
- **No sign-in** — for public servers that need no credential.

### From your plugins

When your agents' plugins use remote MCP servers — the Slack plugin, for
one — they appear under **From your plugins**, with the plugin and the hosts
that use it. A session reaches such a server only once it is an integration:
**Add** fills in the name and address (asking for the address when the
plugin builds it from its host's settings), keeps its tools under the
plugin's own names in Claude Code, and starts it on the hosts that reported
it. **Dismiss** hides it. Hosts report only a server's address and, at most,
the *name* of a header it needs — never a value.

### Services that need your own app

Most services let Storm register itself when you sign in. Some, such as
Slack, don't: they only accept an app you create in their developer
settings. Storm then asks for **your own app**, which you can also choose
from an integration's **⋯ › Use your own app**:

1. Create an app in the service's developer settings.
2. Add the **redirect** Storm shows to the app's redirect URLs. Use **Copy**:
   it must match exactly. On macOS and Android it is
   `storm://oauth/callback`; on Linux and Windows it is
   `http://127.0.0.1:27181/oauth/callback`, a fixed port that must be free
   while you sign in.
3. Paste the app's **client ID**, and its **client secret** if it has one.
   Leave **scopes** empty to ask for the service's own scopes for MCP.

The secret is stored encrypted on your server, never on a host. Signing in
again later reuses the same app.

Storm's own vault is a built-in integration: read only, unless a session is
allowed to write.

## Which hosts may use it

**All hosts** is the default, including hosts enrolled later. **Choose
hosts** limits it to sessions on the hosts you pick — a work integration on
the work machine only, say.

## Which tools are allowed

After the first connection, **Choose tools** decides what agents may call:
they can call only the tools that are on. When the service adds a tool later,
it is recorded but **not** turned on; the integration shows how many new
tools there are until you review the list.

## Disconnecting

**Disconnect** cuts agents off at once. For a sign-in, Storm deletes its
credential and asks the service to revoke it. A token cannot be revoked by
Storm: revoke it at the service too. **Disable** pauses an integration
without forgetting it. **Needs sign-in** or **Needs reconnecting** means the
service has stopped accepting Storm's credential — sign in again.
