# Built-in Model Context Protocol (MCP) Server

`matrix-bot` includes a built-in **Model Context Protocol (MCP)** server compliant with specification `2024-11-05`. This allows external LLMs and AI environments (such as **Claude Desktop**, **Cursor**, **Continue**, or custom MCP clients) to seamlessly connect to your Matrix Bot database and access all productivity tools, live resources, and context prompt templates.

---

## 🏛️ Architecture Overview

```text
┌────────────────────────────────────────────────────────┐
│     External AI Clients (Claude Desktop, Cursor, etc.)  │
└───────────────────────────┬────────────────────────────┘
                            │
              ┌─────────────┴─────────────┐
              │                           │
          STDIO Transport            HTTP / SSE Transport
      (matrix-bot mcp --owner <id>)  (/api/v1/mcp/sse, /messages)
              │                           │
              └─────────────┬─────────────┘
                            │ JSON-RPC 2.0
                            ▼
        ┌─────────────────────────────────────────┐
        │             MCP Service                 │
        │  ├── Tool / Resource / Prompt Registry  │
        │  └── JSON-RPC Request Dispatcher        │
        └───────────────────┬─────────────────────┘
                            │
                            ▼
        ┌─────────────────────────────────────────┐
        │   34 Unified AI Function Tools & DBs    │
        │   (Todos, Finance, Portfolio, Habits,   │
        │    Recurring Bills, Bookmarks, Life)    │
        └─────────────────────────────────────────┘
```

### Key Architectural Characteristics
1. **Zero Duplication**: Reuses the exact same 34 OpenAI-compatible function tool definitions and execution pipelines used by Matrix chat AI.
2. **Dual Transports**:
   - **STDIO**: High-performance line-based JSON-RPC over standard input/output for local CLI subprocesses (e.g. Claude Desktop local runner). Tracing is strictly redirected to `stderr` to prevent stdout protocol corruption.
   - **HTTP/SSE**: Streamable HTTP Server-Sent Events endpoint (`/api/v1/mcp/sse`) and JSON-RPC message receiver (`/api/v1/mcp/messages`), as well as single-endpoint POST (`/api/v1/mcp`).
3. **Multi-tenant / User Scoping**: All tool executions and resource queries are explicitly scoped to the authenticated owner ID.
4. **Real-Time Event Broadcasting**: State mutations executed through MCP emit `SystemEvent` instances via the central `EventBroadcaster`, updating connected Matrix clients and web dashboards instantaneously.

---

## 🧰 Available Capabilities

### 1. Tools (34 Unified AI Tools)
The MCP server exposes all productivity functions as MCP tools:
- **Tasks & Todos**: `get_my_tasks`, `add_task`, `complete_task`, `delete_task`, `update_task`
- **Finance & Expenses**: `get_spending_summary`, `list_transactions`, `record_finance_entry`, `add_expense`, `delete_transaction`
- **Investment Portfolio**: `get_portfolio_summary`, `buy_asset`, `sell_asset`, `update_asset_price`, `deposit_cash`, `withdraw_cash`, `get_cash_balances`, `add_price_alert`, `list_price_alerts`, `delete_price_alert`
- **Recurring Bills**: `list_recurring_bills`, `add_recurring_bill`, `pay_recurring_bill`, `delete_recurring_bill`
- **Habits Tracking**: `list_habits`, `log_habit`, `create_habit`, `delete_habit`
- **Bookmarks**: `list_bookmarks`, `add_bookmark`, `delete_bookmark`
- **Life Grid**: `get_life_profile`, `set_life_profile`, `add_life_milestone`
- **Webhooks**: `list_webhooks`, `trigger_test_webhook`

### 2. Read-Only Resources (`resources/list` & `resources/read`)
Direct access to live structured state without consuming LLM tool-calling tokens:
| Resource URI | Description | MIME Type |
|---|---|---|
| `matrixbot://todos/pending` | Real-time list of all pending todo items | `application/json` |
| `matrixbot://finance/recent` | Recent transactions and monthly category spending | `application/json` |
| `matrixbot://portfolio/summary` | Current portfolio holdings, market values, and cash | `application/json` |
| `matrixbot://habits/today` | Today's active habits and completion statuses | `application/json` |
| `matrixbot://life/profile` | Life grid birthdate, target age, and milestones | `application/json` |

### 3. Prompt Templates (`prompts/list` & `prompts/get`)
Pre-engineered prompt workflows packaged for AI clients:
- **`daily-briefing`**: Summarizes pending tasks, habit adherence for today, and portfolio snapshot.
- **`weekly-retrospective`**: Analyzes the past week's spending patterns, completed tasks, and upcoming bills.
- **`portfolio-review`**: Conducts a health check of asset allocation, cash balances, and price alerts.

---

## 🚀 Client Configuration

### A. Claude Desktop (Local STDIO Transport)

Add the server to your Claude Desktop configuration file:
- **macOS**: `~/Library/Application Support/Claude/claude_desktop_config.json`
- **Windows**: `%APPDATA%\Claude\claude_desktop_config.json`
- **Linux**: `~/.config/Claude/claude_desktop_config.json`

#### Using Prebuilt Binary
```json
{
  "mcpServers": {
    "matrix-bot": {
      "command": "/usr/local/bin/matrix-bot",
      "args": ["mcp", "--owner", "@your_user:matrix.org"],
      "env": {
        "DATABASE_URL": "sqlite:///path/to/data/matrix-bot.db"
      }
    }
  }
}
```

#### Using Cargo Runner (Development)
```json
{
  "mcpServers": {
    "matrix-bot": {
      "command": "cargo",
      "args": ["run", "--manifest-path", "/path/to/matrix-bot/backend/Cargo.toml", "--", "mcp", "--owner", "@your_user:matrix.org"],
      "env": {
        "DATABASE_URL": "sqlite:///path/to/data/matrix-bot.db"
      }
    }
  }
}
```

---

### B. Claude Desktop or Remote Clients (HTTP / SSE Transport)

When running `matrix-bot` as a persistent daemon or remote server:

```json
{
  "mcpServers": {
    "matrix-bot-remote": {
      "url": "https://your-bot-domain.com/api/v1/mcp/sse",
      "headers": {
        "Authorization": "Bearer YOUR_API_OR_SESSION_TOKEN"
      }
    }
  }
}
```

---

### C. Cursor Composer / IDE Integration

In Cursor:
1. Navigate to **Cursor Settings** → **Features** → **MCP**.
2. Click **+ Add New MCP Server**.
3. Choose transport type:
   - **Type**: `command`
   - **Command**: `matrix-bot mcp --owner @your_user:matrix.org`
4. Or configure `.cursor/mcp.json` in your project root:
```json
{
  "mcpServers": {
    "matrix-bot": {
      "command": "matrix-bot",
      "args": ["mcp", "--owner", "@your_user:matrix.org"],
      "env": {
        "DATABASE_URL": "sqlite://./data/matrix-bot.db"
      }
    }
  }
}
```

---

## 💬 Matrix Bot In-Chat Command

You can inspect the status of the MCP server directly within any Matrix room where the bot is present:

```text
!mcp
```

**Response Example:**
> **MCP (Model Context Protocol) Server**
> - **Protocol Version**: `2024-11-05`
> - **Status**: Active
> - **Available Tools**: 34 unified function tools
> - **Resources**: `todos/pending`, `finance/recent`, `portfolio/summary`, `habits/today`, `life/profile`
> - **Prompt Templates**: `daily-briefing`, `weekly-retrospective`, `portfolio-review`
> - **Endpoints**:
>   - SSE: `/api/v1/mcp/sse`
>   - JSON-RPC: `/api/v1/mcp`
>   - STDIO: `matrix-bot mcp [--owner <id>]`
