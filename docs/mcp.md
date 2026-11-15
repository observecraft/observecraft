# MCP server

ObserveCraft runs a [Model Context Protocol](https://modelcontextprotocol.io) server, so AI agents (Claude Code, IDE
assistants, your own agents) can query an organization's telemetry while they investigate an incident or change code.
The agent's client connects over HTTP with an application key, lists the tools and calls them as the model decides.
The design is [ADR-0009](adr/0009-mcp-server.md).

## Connect

The endpoint is your installation's API URL followed by `/api/v1/mcp`, e.g. `https://observecraft.example.com/api/v1/mcp`.
Personal Settings → Application Keys shows it for your installation, with the commands below filled in.

1. Create an application key (Personal Settings → Application Keys). The agent acts as you: your organization, your
   role, your log restrictions. Use a key per agent so you can revoke one without the others.
2. Add the server to the client.

Claude Code:

```bash
claude mcp add --transport http observecraft https://observecraft.example.com/api/v1/mcp \
  --header "Authorization: Bearer ocak_…"
```

Other clients take a JSON configuration of this shape (the file and the top-level key vary by client):

```json
{
  "mcpServers": {
    "observecraft": {
      "type": "http",
      "url": "https://observecraft.example.com/api/v1/mcp",
      "headers": { "Authorization": "Bearer ocak_…" }
    }
  }
}
```

Clients and scripts already set up for the Datadog-compatible management API can send the key as
`DD-APPLICATION-KEY` instead (with `DD-API-KEY`, if sent, being an ingest key of the same organization).

## Tools

Every tool only reads. Times are relative to now (`15m`, `1h`, `7d`) or epoch milliseconds.

| Tool | What it returns |
|---|---|
| `search_logs` | Log events, newest first: time, service, status, host, message, tags and attributes; pages with `cursor` |
| `aggregate_logs` | Log counts (or a measure: avg, sum, percentiles, cardinality) grouped by up to 3 facets |
| `query_metrics` | Metric queries (`avg:system.cpu.user{env:prod} by {host}`): per series last, min, max, average and points |
| `search_metrics` | Metric names reported in the last day, with kind, unit, description and tag keys |
| `search_spans` | Spans matching a search (`service:checkout status:error @duration:>1s`) |
| `get_trace` | One trace's spans in start order |
| `list_services` | APM services with requests, error rate and latency |
| `search_monitors`, `get_monitor` | Monitors with their state; one monitor's query, thresholds and groups |
| `search_incidents`, `get_incident` | Incidents; one incident with its timeline |
| `search_events` | Events (deployments, alerts, Agent and API events) |
| `search_hosts` | Hosts with cloud, OS, env, CPU and memory use, monitors and APM errors |
| `search_kubernetes` | Kubernetes resources of a kind (pods, deployments, nodes, …) with their status |
| `search_dashboards`, `get_dashboard` | Dashboards; one dashboard's widgets and their queries |
| `search_error_issues` | Error Tracking issues, most frequent first |
| `search_security_signals`, `get_security_signal` | Cloud SIEM signals; one signal with its rule and matches |
| `search_vulnerabilities` | Cloud Security vulnerabilities of hosts and container images |
| `search_code_security_findings` | Static code findings (SAST, secrets, IaC) |

Log and span queries use the explorer's search syntax; metric queries the metric query syntax.

## Limits

- A time range covers at most 31 days.
- Rows per call are capped (logs 100, spans and events 50, metric series 40 with up to 120 points, trace spans 200);
  log messages are cut at 500 characters and attribute values at 200.
- A result longer than 50 000 characters is cut, with a note asking to narrow the call.
- A call is abandoned after 30 seconds, and a user may make 50 calls per 10 seconds on each server replica. The
  organization's ClickHouse query limits apply as in the UI.
- Mistakes in arguments come back as tool errors (`isError`) the model can correct.

## Audit

Each tool call is in the organization's audit trail (Organization Settings → Audit Trail) as `mcp.tool_call`, with the
user, the tool and its arguments.

## Protocol

Streamable HTTP, JSON answers only (no SSE streams, no sessions): any server replica answers any request. Protocol
versions `2026-07-28` (stateless, with `server/discover`) and `2025-11-25`, `2025-06-18`, `2025-03-26` (`initialize`
handshake). Methods: `initialize`, `server/discover`, `ping`, `tools/list`, `tools/call`. `GET` answers 405. Requests
with a browser `Origin` other than the installation's own are refused.

Try it with curl:

```bash
curl -s https://observecraft.example.com/api/v1/mcp \
  -H "Authorization: Bearer ocak_…" -H "Content-Type: application/json" -H "MCP-Protocol-Version: 2025-11-25" \
  -d '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"list_services","arguments":{"from":"1h"}}}'
```
