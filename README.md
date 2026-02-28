# zenmoney-mcp

MCP server wrapping the [ZenMoney](https://zenmoney.ru/) personal finance API.

Exposes tools (sync, read, search, write, bulk) via the [Model Context Protocol](https://modelcontextprotocol.io/) over stdio, allowing LLM assistants to interact with your ZenMoney data.

> **Caution:** Using MCP always carries a certain risk to your data, especially in the early stages of adoption. You or other users who decide to try MCP with ZenMoney may want to make backups (including daily backups to Telegram) and restore them to test accounts. See [zentable.ru/backup](https://zentable.ru/backup) and [zentable.ru/backup/restore](https://zentable.ru/backup/restore).
>
> **Important:** When creating a test account, do it from a different phone or a cloned app. Otherwise, your bank synchronizations may break when you switch back to your main account.

## Installation

```bash
cargo install zenmoney-mcp
```

## Getting a Token

To use this server, you need a ZenMoney API access token. You can obtain one by creating an application at [ZenMoney API](https://developers.zenmoney.ru/) and following the OAuth2 flow, or by extracting the token from the ZenMoney web app (see the [ZenMoney API documentation](https://github.com/zenmoney/ZenPlugins/wiki/ZenMoney-API) for details).

## Usage

```bash
ZENMONEY_TOKEN=<your-token> zenmoney-mcp
```

The server performs an initial sync on startup, then serves MCP tools over stdio.

## Claude Desktop Integration

Add the following to your Claude Desktop config file:

| OS | Config path |
|----|-------------|
| macOS | `~/Library/Application Support/Claude/claude_desktop_config.json` |
| Windows | `%APPDATA%\Claude\claude_desktop_config.json` |
| Linux | `~/.config/Claude/claude_desktop_config.json` |

```json
{
  "mcpServers": {
    "zenmoney": {
      "command": "zenmoney-mcp",
      "env": {
        "ZENMONEY_TOKEN": "your-token-here"
      }
    }
  }
}
```

Replace `your-token-here` with your ZenMoney API token, then restart Claude Desktop.

## Tools

### Sync
- `sync` — incremental sync with ZenMoney server
- `full_sync` — full re-download of all data

### Read
- `list_accounts` — list financial accounts (filter by active)
- `list_transactions` — list transactions with filters (date, account, tag, payee, amount, type, uncategorized, sort)
- `list_tags` — list category tags
- `list_merchants` — list merchants
- `list_budgets` — list monthly budgets
- `list_reminders` — list recurring reminders
- `list_instruments` — list currency instruments

### Search
- `find_account` — find account by title
- `find_tag` — find tag by title
- `suggest_category` — suggest category for a transaction (no confidence scores)
- `get_instrument` — get instrument by ID

### Write
- `create_transaction` — create a transaction (expense/income/transfer with auto-resolved currency)
- `update_transaction` — update an existing transaction by ID
- `delete_transaction` — delete a transaction (returns details of what was deleted)
- `bulk_operations` — batch create/update/delete in a single call

## Usage Scenarios

### Deduplication of Transactions

Find and remove duplicate transactions by searching for matching payee, amount, and date:

1. `list_transactions` with date range + payee/amount filters to find potential duplicates
2. Review the results and identify true duplicates
3. `delete_transaction` to remove duplicates — the response shows full details of the deleted transaction for confirmation

### Setting Up Correct Categories

Find uncategorized transactions and assign categories:

1. `list_transactions(uncategorized: true)` to find all transactions without tags
2. For each: `suggest_category(payee: "...")` to get ZenMoney's suggestion
3. `update_transaction(id: "...", tag_ids: ["..."])` to apply the category
4. Or use `bulk_operations` to categorize many transactions at once

### Monthly Financial Report

Generate a summary of income, expenses, and budgets for a month:

1. `list_transactions(date_from: "2025-02-01", date_to: "2025-02-28", transaction_type: "expense", sort: "desc")` for all expenses
2. `list_transactions(date_from: "2025-02-01", date_to: "2025-02-28", transaction_type: "income")` for all income
3. `list_budgets(month: "2025-02")` for budget targets
4. `list_accounts(active_only: true)` for current account balances

## License

MIT OR Apache-2.0
