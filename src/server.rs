//! MCP server implementation wrapping the `zenmoney-rs` client.
//!
//! Uses `rmcp` macros to expose ZenMoney API operations as MCP tools.

extern crate alloc;

use alloc::sync::Arc;

use rmcp::handler::server::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{CallToolResult, Content, ServerCapabilities, ServerInfo};
use rmcp::{ErrorData as McpError, ServerHandler, tool, tool_handler, tool_router};
use zenmoney_rs::models::{
    AccountId, InstrumentId, MerchantId, NaiveDate, SuggestRequest, TagId, Transaction,
    TransactionId, UserId,
};
use zenmoney_rs::storage::FileStorage;
use zenmoney_rs::zen_money::{TransactionFilter, ZenMoney};

use chrono::{DateTime, Utc};

use crate::params::{
    CreateTransactionParams, DeleteTransactionParams, FindAccountParams, FindTagParams,
    GetInstrumentParams, ListAccountsParams, ListBudgetsParams, ListTransactionsParams,
    SuggestCategoryParams,
};
use crate::response::{
    AccountResponse, BudgetResponse, InstrumentResponse, LookupMaps, MerchantResponse,
    ReminderResponse, SuggestResponse, TagResponse, TransactionResponse, build_lookup_maps,
};

/// MCP server wrapping the ZenMoney personal finance API.
#[derive(Clone)]
pub(crate) struct ZenMoneyMcpServer {
    /// Inner ZenMoney client (shared via Arc).
    client: Arc<ZenMoney<FileStorage>>,
    /// Tool router for dispatching MCP tool calls.
    tool_router: ToolRouter<Self>,
}

impl core::fmt::Debug for ZenMoneyMcpServer {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ZenMoneyMcpServer").finish_non_exhaustive()
    }
}

/// Converts a [`zenmoney_rs::error::ZenMoneyError`] into an MCP internal error.
#[allow(clippy::needless_pass_by_value, reason = "map_err passes by value")]
fn zen_err(err: zenmoney_rs::error::ZenMoneyError) -> McpError {
    McpError::internal_error(err.to_string(), None)
}

/// Parses a date string in `YYYY-MM-DD` format.
fn parse_date(date_str: &str) -> Result<NaiveDate, McpError> {
    NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
        .map_err(|err| McpError::invalid_params(format!("invalid date '{date_str}': {err}"), None))
}

/// Serializes a value to a pretty-printed JSON string for tool output.
fn to_json_text<T: serde::Serialize>(value: &T) -> Result<String, McpError> {
    serde_json::to_string_pretty(value).map_err(|err| {
        McpError::internal_error(format!("failed to serialize response: {err}"), None)
    })
}

/// Creates a successful tool result containing JSON text.
fn json_result<T: serde::Serialize>(value: &T) -> Result<CallToolResult, McpError> {
    let text = to_json_text(value)?;
    Ok(CallToolResult::success(vec![Content::text(text)]))
}

/// Formats an [`AccountType`](zenmoney_rs::models::AccountType) variant as a human-readable string.
pub(crate) const fn account_type_label(kind: zenmoney_rs::models::AccountType) -> &'static str {
    match kind {
        zenmoney_rs::models::AccountType::Cash => "Cash",
        zenmoney_rs::models::AccountType::CreditCard => "CreditCard",
        zenmoney_rs::models::AccountType::Checking => "Checking",
        zenmoney_rs::models::AccountType::Loan => "Loan",
        zenmoney_rs::models::AccountType::Deposit => "Deposit",
        zenmoney_rs::models::AccountType::EMoney => "EMoney",
        zenmoney_rs::models::AccountType::Debt => "Debt",
    }
}

#[tool_router]
impl ZenMoneyMcpServer {
    /// Creates a new MCP server with the given ZenMoney client.
    pub(crate) fn new(client: ZenMoney<FileStorage>) -> Self {
        Self {
            client: Arc::new(client),
            tool_router: Self::tool_router(),
        }
    }

    /// Builds lookup maps from current storage for enriching responses.
    async fn lookup_maps(&self) -> Result<LookupMaps, McpError> {
        let accounts = self.client.accounts().await.map_err(zen_err)?;
        let tags = self.client.tags().await.map_err(zen_err)?;
        let instruments = self.client.instruments().await.map_err(zen_err)?;
        Ok(build_lookup_maps(&accounts, &tags, &instruments))
    }

    // ── Sync tools ──────────────────────────────────────────────────

    /// Performs an incremental sync with the ZenMoney server.
    #[tool(
        description = "Perform an incremental sync with the ZenMoney server, fetching only changes since the last sync"
    )]
    async fn sync(&self) -> Result<CallToolResult, McpError> {
        let _response = self.client.sync().await.map_err(zen_err)?;
        Ok(CallToolResult::success(vec![Content::text(
            "Sync completed successfully",
        )]))
    }

    /// Performs a full sync, clearing local data and re-downloading everything.
    #[tool(
        description = "Perform a full sync, clearing all local data and re-downloading everything from the ZenMoney server"
    )]
    async fn full_sync(&self) -> Result<CallToolResult, McpError> {
        let _response = self.client.full_sync().await.map_err(zen_err)?;
        Ok(CallToolResult::success(vec![Content::text(
            "Full sync completed successfully",
        )]))
    }

    // ── Read tools ──────────────────────────────────────────────────

    /// Lists all accounts (or only active ones).
    #[tool(
        description = "List financial accounts. Set active_only=true to exclude archived accounts"
    )]
    async fn list_accounts(
        &self,
        params: Parameters<ListAccountsParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let accounts = if params.0.active_only {
            self.client.active_accounts().await.map_err(zen_err)?
        } else {
            self.client.accounts().await.map_err(zen_err)?
        };
        let result: Vec<AccountResponse> = accounts
            .iter()
            .map(|acc| AccountResponse::from_account(acc, &maps))
            .collect();
        json_result(&result)
    }

    /// Lists transactions with optional filtering.
    #[tool(
        description = "List transactions with optional filters: date range, account, tag, payee, merchant, amount range, and result limit"
    )]
    async fn list_transactions(
        &self,
        params: Parameters<ListTransactionsParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let mut filter = TransactionFilter::new();

        if let Some(date_from_str) = params.0.date_from.as_deref() {
            filter.date_from = Some(parse_date(date_from_str)?);
        }
        if let Some(date_to_str) = params.0.date_to.as_deref() {
            filter.date_to = Some(parse_date(date_to_str)?);
        }
        if let Some(account_id) = params.0.account_id.as_ref() {
            filter = filter.account(AccountId::new(account_id.clone()));
        }
        if let Some(tag_id) = params.0.tag_id.as_ref() {
            filter = filter.tag(TagId::new(tag_id.clone()));
        }
        if let Some(payee_str) = params.0.payee.as_ref() {
            filter = filter.payee(payee_str.clone());
        }
        if let Some(merchant_id) = params.0.merchant_id.as_ref() {
            filter = filter.merchant(MerchantId::new(merchant_id.clone()));
        }
        if let Some(min) = params.0.min_amount {
            filter.min_amount = Some(min);
        }
        if let Some(max) = params.0.max_amount {
            filter.max_amount = Some(max);
        }

        let mut transactions = self
            .client
            .filter_transactions(&filter)
            .await
            .map_err(zen_err)?;

        if let Some(limit) = params.0.limit {
            transactions.truncate(limit);
        }

        let result: Vec<TransactionResponse> = transactions
            .iter()
            .map(|tx| TransactionResponse::from_transaction(tx, &maps))
            .collect();
        json_result(&result)
    }

    /// Lists all category tags.
    #[tool(description = "List all transaction category tags")]
    async fn list_tags(&self) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let tags = self.client.tags().await.map_err(zen_err)?;
        let result: Vec<TagResponse> = tags
            .iter()
            .map(|tag| TagResponse::from_tag(tag, &maps))
            .collect();
        json_result(&result)
    }

    /// Lists all merchants.
    #[tool(description = "List all merchants/payees")]
    async fn list_merchants(&self) -> Result<CallToolResult, McpError> {
        let merchants = self.client.merchants().await.map_err(zen_err)?;
        let result: Vec<MerchantResponse> = merchants
            .iter()
            .map(MerchantResponse::from_merchant)
            .collect();
        json_result(&result)
    }

    /// Lists budgets, optionally filtered by month.
    #[tool(description = "List monthly budgets. Optionally filter by month (format: YYYY-MM)")]
    async fn list_budgets(
        &self,
        params: Parameters<ListBudgetsParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let budgets = self.client.budgets().await.map_err(zen_err)?;

        let filtered_budgets: Vec<_> = if let Some(month_str) = params.0.month.as_deref() {
            let month_prefix = format!("{month_str}-01");
            let month_date = parse_date(&month_prefix)?;
            budgets
                .into_iter()
                .filter(|budget| budget.date == month_date)
                .collect()
        } else {
            budgets
        };

        let result: Vec<BudgetResponse> = filtered_budgets
            .iter()
            .map(|budget| BudgetResponse::from_budget(budget, &maps))
            .collect();
        json_result(&result)
    }

    /// Lists all reminders.
    #[tool(description = "List all recurring transaction reminders")]
    async fn list_reminders(&self) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let reminders = self.client.reminders().await.map_err(zen_err)?;
        let result: Vec<ReminderResponse> = reminders
            .iter()
            .map(|rem| ReminderResponse::from_reminder(rem, &maps))
            .collect();
        json_result(&result)
    }

    /// Lists all currency instruments.
    #[tool(description = "List all currency instruments with their exchange rates")]
    async fn list_instruments(&self) -> Result<CallToolResult, McpError> {
        let instruments = self.client.instruments().await.map_err(zen_err)?;
        let result: Vec<InstrumentResponse> = instruments
            .iter()
            .map(InstrumentResponse::from_instrument)
            .collect();
        json_result(&result)
    }

    // ── Search tools ────────────────────────────────────────────────

    /// Finds an account by title.
    #[tool(description = "Find an account by title (case-insensitive search)")]
    async fn find_account(
        &self,
        params: Parameters<FindAccountParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let account = self
            .client
            .find_account_by_title(&params.0.title)
            .await
            .map_err(zen_err)?;
        if let Some(acc) = account.as_ref() {
            let result = AccountResponse::from_account(acc, &maps);
            json_result(&result)
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No account found with title '{}'",
                params.0.title
            ))]))
        }
    }

    /// Finds a tag by title.
    #[tool(description = "Find a category tag by title (case-insensitive search)")]
    async fn find_tag(
        &self,
        params: Parameters<FindTagParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let tag = self
            .client
            .find_tag_by_title(&params.0.title)
            .await
            .map_err(zen_err)?;
        if let Some(found_tag) = tag.as_ref() {
            let result = TagResponse::from_tag(found_tag, &maps);
            json_result(&result)
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No tag found with title '{}'",
                params.0.title
            ))]))
        }
    }

    /// Suggests a category for a transaction.
    #[tool(
        description = "Suggest a category tag for a transaction based on payee name and/or comment"
    )]
    async fn suggest_category(
        &self,
        params: Parameters<SuggestCategoryParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let request = SuggestRequest {
            payee: params.0.payee,
            comment: params.0.comment,
        };
        let response = self.client.suggest(&request).await.map_err(zen_err)?;
        let result = SuggestResponse::from_suggest(&response, &maps);
        json_result(&result)
    }

    /// Gets a specific instrument by ID.
    #[tool(description = "Get a specific currency instrument by its numeric ID")]
    async fn get_instrument(
        &self,
        params: Parameters<GetInstrumentParams>,
    ) -> Result<CallToolResult, McpError> {
        let instrument = self
            .client
            .instrument(InstrumentId::new(params.0.id))
            .await
            .map_err(zen_err)?;
        if let Some(instr) = instrument.as_ref() {
            let result = InstrumentResponse::from_instrument(instr);
            json_result(&result)
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "No instrument found with ID {}",
                params.0.id
            ))]))
        }
    }

    // ── Write tools ─────────────────────────────────────────────────

    /// Creates a new transaction.
    #[tool(
        description = "Create a new financial transaction. Requires date, accounts, amounts, and instrument IDs. Optionally specify tags, payee, and comment"
    )]
    async fn create_transaction(
        &self,
        params: Parameters<CreateTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let date = parse_date(&params.0.date)?;
        let now: DateTime<Utc> = Utc::now();
        let transaction_id = uuid::Uuid::new_v4().to_string();

        let tag_ids: Option<Vec<TagId>> = params
            .0
            .tag_ids
            .map(|ids| ids.into_iter().map(TagId::new).collect());

        let tx = Transaction {
            id: TransactionId::new(transaction_id.clone()),
            changed: now,
            created: now,
            user: UserId::new(0),
            deleted: false,
            hold: None,
            income_instrument: InstrumentId::new(params.0.income_instrument),
            income_account: AccountId::new(params.0.income_account),
            income: params.0.income,
            outcome_instrument: InstrumentId::new(params.0.outcome_instrument),
            outcome_account: AccountId::new(params.0.outcome_account),
            outcome: params.0.outcome,
            tag: tag_ids,
            merchant: None,
            payee: params.0.payee,
            original_payee: None,
            comment: params.0.comment,
            date,
            mcc: None,
            reminder_marker: None,
            op_income: None,
            op_income_instrument: None,
            op_outcome: None,
            op_outcome_instrument: None,
            latitude: None,
            longitude: None,
            income_bank_id: None,
            outcome_bank_id: None,
            qr_code: None,
            source: None,
            viewed: None,
        };

        let _response = self
            .client
            .push_transactions(vec![tx])
            .await
            .map_err(zen_err)?;

        Ok(CallToolResult::success(vec![Content::text(format!(
            "Transaction created successfully with ID: {transaction_id}"
        ))]))
    }

    /// Deletes a transaction by ID.
    #[tool(description = "Delete a transaction by its ID")]
    async fn delete_transaction(
        &self,
        params: Parameters<DeleteTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let tx_id = TransactionId::new(params.0.id.clone());
        let _response = self
            .client
            .delete_transactions(&[tx_id])
            .await
            .map_err(zen_err)?;
        Ok(CallToolResult::success(vec![Content::text(format!(
            "Transaction '{}' deleted successfully",
            params.0.id
        ))]))
    }
}

#[tool_handler]
impl ServerHandler for ZenMoneyMcpServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some(
                "ZenMoney personal finance MCP server. \
                 Use sync/full_sync to fetch data, then query accounts, \
                 transactions, tags, budgets, and more."
                    .into(),
            ),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}
