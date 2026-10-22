---
title: Plugins
description: What an agent's plugins and their MCP servers do in a Storm session, agent by agent.
---

Agents install **plugins** that bring skills, commands, hooks and often an
**MCP server**: the Slack plugin, Playwright, a cloud provider's tools. In a
Storm session a plugin works as it does without Storm: its local servers
run on the host, and its remote ones sign in the way the agent signs them
in (decision 132). Storm adds your
[integrations](/docs/agents/integrations/) beside them.

A remote server can also be an integration, signed in once on your server
and given to the hosts you choose, with Storm's audit and its question
before each tool. Settings › Integrations lists your plugins' remote
servers under **From your plugins** for that.

## Agent by agent

| Agent | A plugin's local servers | A plugin's remote servers | Also left out |
|---|---|---|---|
| **Claude Code** | Run in the session, as they do without Storm | Load as Claude Code loads them; also suggested as integrations | Nothing: your own `~/.claude.json` servers, a repository's `.mcp.json` (after Claude Code asks you) and claude.ai connectors load as usual, beside your integrations |
| **Codex** | Run in the session | Load as Codex loads them; also suggested as integrations | Nothing: your own `mcp_servers`, ChatGPT apps and plugins load as usual, and a repository's `.codex/` settings once you trust it in Codex |
| **OpenCode** | — (its plugins are JavaScript, not MCP servers) | — | A repository's own config: it could turn off the question OpenCode asks before each Storm tool. Your global config, and its servers, load as usual |
| **Cursor**, **Command Code** | — | — | They run as terminals with no Storm tools until Storm has checked them with a signed-in account |

An integration added from a plugin keeps the plugin's tool names in Claude
Code, so the plugin's commands and hooks still find its tools.

A server that needs one of the plugin's own options (`${user_config.…}`) is
skipped too: Storm can't read those values.

## Storm's servers only

A host can give its sessions Storm's integrations and nothing else, for a
machine that runs code you don't trust. In its `runtime.toml`:

```toml
agent_mcp = "storm-only"
```

Claude Code and Codex sessions then load only Storm's servers (and, unless
`plugin_mcp = "off"`, their plugins' local ones), and Codex keeps every
workspace untrusted.

## Turning plugin servers off on a host

In the host's `runtime.toml`:

```toml
plugin_mcp = "off"
```

It applies with `agent_mcp = "storm-only"`: every plugin's local servers are
then skipped as well, in Claude Code and in Codex, and each session says
so.

## What Storm reads

From a plugin, Storm reads which servers it declares and how they start. To
suggest a remote server, a host reports its **address** and, at most, the
**name** of a header it fills from the host's environment — never a header's
value, an environment value, or a vendor's sign-in app. From your own and a
repository's config files, Storm reads **server names only**, to name them.

## Adding Slack

The Slack plugin's server works in a Claude Code session as it does
without Storm, signed in on that host. To sign in once for every host
instead, add it as an integration. Slack does not let other apps register
themselves, so you add it with **your own Slack app**:

> These steps haven't been tried against Slack's own server yet. If Slack
> refuses Storm's redirect, or its server refuses your app's sign-in, please
> [open an issue](https://github.com/dewanshDT/Storm/issues).

1. In **Settings › Integrations**, find Slack under **From your plugins** and
   choose **Add** (or add `https://mcp.slack.com/mcp` yourself), with
   **Sign in**.
2. Storm asks for your own app and shows the **redirect** to register.
3. At [api.slack.com/apps](https://api.slack.com/apps), create an app in your
   workspace. Under **OAuth & Permissions**, add the redirect Storm showed
   (use **Copy**) to the redirect URLs.
4. Back in Storm, paste the app's **Client ID** and **Client Secret** (from
   **Basic Information**). Leave scopes empty: Storm asks for the ones
   Slack's MCP server publishes.
5. Sign in in your browser. The integration starts on the hosts whose agents
   use the Slack plugin; widen it from **⋯ › Choose hosts**.

The plugin carries Slack's own app for Claude Code; Storm never uses another
product's app.
