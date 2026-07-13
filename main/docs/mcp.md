# Doc Search, Token Saving, And Custom MCPs

Cntx Code ships two built-in capabilities as on-demand MCP servers, and lets you
register any custom MCP server from YAML or the CLI.

## Built-In Doc Search (Context7)

Context7 resolves library names and fetches up-to-date documentation for them. It is
configured by default as the `context7` server and spawned only when you ask for it,
so it never slows down ordinary prompts. It exposes:

- `resolve-library-id` - resolves a library name to a Context7 id
- `query-docs` - fetches documentation for a given library id

Requirements: Node.js 18+ (the server runs via `npx -y @upstash/context7-mcp`).
An optional `CONTEXT7_API_KEY` raises rate limits; anonymous use is allowed.

```bash
cntx mcp list
cntx mcp tools context7
```

## Built-In Token Saving (Headroom)

Headroom compresses large tool outputs, logs, and file contents by 60-95% before they
reach the model, and retrieves the originals later by hash. It is configured by default
as the `headroom` server. It exposes:

- `headroom_compress` - compress content, returns compressed text and a hash
- `headroom_retrieve` - retrieve the original uncompressed content by hash
- `headroom_stats` - session compression statistics

Requirements: install with `pip install "headroom-ai[mcp]"`. Local compression needs no key.

```bash
cntx mcp tools headroom
```

## Why Servers Are Spawned On Demand

MCP servers are local subprocesses. Cntx Code spawns one only when you run
`cntx mcp tools <name>` or when an agent loop calls a tool. A normal prompt never
launches a subprocess, so startup stays fast and there is no background noise.

> **Note:** The autonomous agent loop that invokes MCP tools during a normal prompt
> is not yet implemented. Today `cntx mcp tools <name>` is a manual inspection tool
> you use to connect to a server and list the tools it exposes. Automatic
> invocation of those tools by the assistant during `--tool-use` is planned.

## Adding A Custom MCP Server

```bash
cntx mcp add filesystem npx \
  --arg -y --arg @modelcontextprotocol/server-filesystem \
  --env ROOT=/Users/you/projects
```

Import several servers from YAML:

```yaml
# mcp-servers.yaml
servers:
  - name: filesystem
    command: npx
    args: ["-y", "@modelcontextprotocol/server-filesystem", "/Users/you/projects"]
    enabled: true
  - name: my-server
    command: ./bin/my-mcp-server
    env:
      LOG_LEVEL: debug
    enabled: false
```

```bash
cntx mcp add --file mcp-servers.yaml
```

## Managing Servers

```bash
cntx mcp list
cntx mcp enable filesystem
cntx mcp disable filesystem
cntx mcp remove filesystem
```

Built-in servers (`context7`, `headroom`) cannot be removed; disable them instead.

## In The Interactive Shell

```text
/mcp         list configured servers
```

Use `cntx mcp tools <name>` to connect and list exposed tools.