# zenmoney-mcp

MCP server wrapping the [ZenMoney](https://zenmoney.ru/) personal finance API.

Exposes 15 tools (sync, read, search, write) via the [Model Context Protocol](https://modelcontextprotocol.io/) over stdio, allowing LLM assistants to interact with your ZenMoney data.

## Usage

```bash
ZENMONEY_TOKEN=<your-token> zenmoney-mcp
```

The server performs an initial sync on startup, then serves MCP tools over stdio.

## Tools

### Sync
- `sync` — incremental sync with ZenMoney server
- `full_sync` — full re-download of all data

### Read
- `list_accounts` — list financial accounts
- `list_transactions` — list transactions with filters (date, account, tag, payee, amount, etc.)
- `list_tags` — list category tags
- `list_merchants` — list merchants
- `list_budgets` — list monthly budgets
- `list_reminders` — list recurring reminders
- `list_instruments` — list currency instruments

### Search
- `find_account` — find account by title
- `find_tag` — find tag by title
- `suggest_category` — suggest category for a transaction
- `get_instrument` — get instrument by ID

### Write
- `create_transaction` — create a new transaction
- `delete_transaction` — delete a transaction

## License

MIT OR Apache-2.0
