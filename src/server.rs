//! MCP server implementation wrapping the `zenmoney-rs` client.
//!
//! Uses `rmcp` macros to expose ZenMoney API operations as MCP tools.

extern crate alloc;

use alloc::sync::Arc;
use std::collections::HashMap;
use std::sync::Mutex;

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
    BulkOperation, BulkOperationsParams, CreateTransactionParams, DeleteTransactionParams,
    ExecuteBulkParams, FindAccountParams, FindTagParams, GetInstrumentParams, ListAccountsParams,
    ListBudgetsParams, ListTransactionsParams, SortDirection, SuggestCategoryParams,
    TransactionType, UpdateTransactionParams,
};
use crate::response::{
    AccountResponse, BudgetResponse, BulkOperationsResponse, DeletedTransactionResponse,
    InstrumentResponse, LookupMaps, MerchantResponse, PrepareResponse, ReminderResponse,
    SuggestResponse, TagResponse, TransactionResponse, build_lookup_maps,
};

/// Holds the validated, ready-to-execute bulk operations.
struct PreparedBulk {
    /// Transactions to create or update.
    to_push: Vec<Transaction>,
    /// Transaction IDs to delete.
    to_delete: Vec<TransactionId>,
    /// Number of create operations.
    created_count: usize,
    /// Number of update operations.
    updated_count: usize,
    /// Enriched preview of create/update transactions.
    preview: Vec<TransactionResponse>,
    /// Enriched preview of transactions to be deleted.
    deleted_preview: Vec<TransactionResponse>,
}

/// MCP server wrapping the ZenMoney personal finance API.
#[derive(Clone)]
pub(crate) struct ZenMoneyMcpServer {
    /// Inner ZenMoney client (shared via Arc).
    client: Arc<ZenMoney<FileStorage>>,
    /// Tool router for dispatching MCP tool calls.
    tool_router: ToolRouter<Self>,
    /// In-memory store of prepared bulk operations awaiting execution.
    preparations: Arc<Mutex<HashMap<String, PreparedBulk>>>,
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

/// Resolves the instrument ID for an account, using an explicit override if provided.
///
/// Returns `explicit` if `Some`, otherwise looks up the account's instrument from the maps.
/// Returns an error if neither is available.
fn resolve_instrument(
    maps: &LookupMaps,
    account_id: &str,
    explicit: Option<i32>,
) -> Result<InstrumentId, McpError> {
    if let Some(id) = explicit {
        return Ok(InstrumentId::new(id));
    }
    maps.account_instrument(account_id)
        .map(InstrumentId::new)
        .ok_or_else(|| {
            McpError::invalid_params(
                format!("cannot resolve instrument for account '{account_id}'; provide instrument_id explicitly"),
                None,
            )
        })
}

/// Classifies a transaction as expense, income, or transfer based on its amounts and accounts.
fn classify_transaction(tx: &Transaction) -> TransactionType {
    let different_accounts = tx.outcome_account.as_inner() != tx.income_account.as_inner();
    if tx.outcome > 0.0 && tx.income > 0.0 && different_accounts {
        TransactionType::Transfer
    } else if tx.income > 0.0 && (tx.outcome == 0.0 || !different_accounts) {
        TransactionType::Income
    } else {
        TransactionType::Expense
    }
}

/// Filters transactions in-place by transaction type, if specified.
fn filter_by_transaction_type(
    transactions: &mut Vec<Transaction>,
    filter_type: Option<&TransactionType>,
) {
    match filter_type {
        Some(&TransactionType::Expense) => {
            transactions.retain(|tx| matches!(classify_transaction(tx), TransactionType::Expense));
        }
        Some(&TransactionType::Income) => {
            transactions.retain(|tx| matches!(classify_transaction(tx), TransactionType::Income));
        }
        Some(&TransactionType::Transfer) => {
            transactions.retain(|tx| matches!(classify_transaction(tx), TransactionType::Transfer));
        }
        None => {}
    }
}

/// Returns `true` if the transaction has no category tags.
fn is_uncategorized(tx: &Transaction) -> bool {
    tx.tag.as_ref().is_none_or(Vec::is_empty)
}

/// Resolved account/amount/instrument fields for building a transaction.
struct ResolvedSides {
    /// Outcome (source) account.
    outcome_account: AccountId,
    /// Outcome amount.
    outcome: f64,
    /// Outcome currency instrument.
    outcome_instrument: InstrumentId,
    /// Income (destination) account.
    income_account: AccountId,
    /// Income amount.
    income: f64,
    /// Income currency instrument.
    income_instrument: InstrumentId,
}

/// Resolves outcome/income sides from the simplified create parameters.
fn resolve_sides(
    params: &CreateTransactionParams,
    maps: &LookupMaps,
) -> Result<ResolvedSides, McpError> {
    match params.transaction_type {
        TransactionType::Expense => {
            let instrument = resolve_instrument(maps, &params.account_id, params.instrument_id)?;
            Ok(ResolvedSides {
                outcome_account: AccountId::new(params.account_id.clone()),
                outcome: params.amount,
                outcome_instrument: instrument,
                income_account: AccountId::new(params.account_id.clone()),
                income: 0.0_f64,
                income_instrument: instrument,
            })
        }
        TransactionType::Income => {
            let instrument = resolve_instrument(maps, &params.account_id, params.instrument_id)?;
            Ok(ResolvedSides {
                outcome_account: AccountId::new(params.account_id.clone()),
                outcome: 0.0_f64,
                outcome_instrument: instrument,
                income_account: AccountId::new(params.account_id.clone()),
                income: params.amount,
                income_instrument: instrument,
            })
        }
        TransactionType::Transfer => {
            let to_account_id = params.to_account_id.as_ref().ok_or_else(|| {
                McpError::invalid_params(
                    "to_account_id is required for transfer transactions".to_owned(),
                    None,
                )
            })?;
            let from_instrument =
                resolve_instrument(maps, &params.account_id, params.instrument_id)?;
            let to_instrument = resolve_instrument(maps, to_account_id, params.to_instrument_id)?;
            let to_amount = params.to_amount.unwrap_or(params.amount);
            Ok(ResolvedSides {
                outcome_account: AccountId::new(params.account_id.clone()),
                outcome: params.amount,
                outcome_instrument: from_instrument,
                income_account: AccountId::new(to_account_id.clone()),
                income: to_amount,
                income_instrument: to_instrument,
            })
        }
    }
}

/// Builds a [`Transaction`] from simplified [`CreateTransactionParams`].
fn build_transaction(
    params: CreateTransactionParams,
    maps: &LookupMaps,
) -> Result<Transaction, McpError> {
    let date = parse_date(&params.date)?;
    let now: DateTime<Utc> = Utc::now();
    let transaction_id = uuid::Uuid::new_v4().to_string();

    let tag_ids: Option<Vec<TagId>> = params
        .tag_ids
        .as_ref()
        .map(|ids| ids.iter().cloned().map(TagId::new).collect());

    let sides = resolve_sides(&params, maps)?;

    Ok(Transaction {
        id: TransactionId::new(transaction_id),
        changed: now,
        created: now,
        user: UserId::new(0),
        deleted: false,
        hold: None,
        income_instrument: sides.income_instrument,
        income_account: sides.income_account,
        income: sides.income,
        outcome_instrument: sides.outcome_instrument,
        outcome_account: sides.outcome_account,
        outcome: sides.outcome,
        tag: tag_ids,
        merchant: None,
        payee: params.payee,
        original_payee: None,
        comment: params.comment,
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
    })
}

/// Applies [`UpdateTransactionParams`] to an existing [`Transaction`].
fn apply_update(
    tx: &mut Transaction,
    params: UpdateTransactionParams,
    maps: &LookupMaps,
) -> Result<(), McpError> {
    if let Some(date_str) = params.date.as_deref() {
        tx.date = parse_date(date_str)?;
    }

    if let Some(tag_ids) = params.tag_ids {
        tx.tag = Some(tag_ids.into_iter().map(TagId::new).collect());
    }

    if let Some(payee) = params.payee {
        tx.payee = if payee.is_empty() { None } else { Some(payee) };
    }

    if let Some(comment) = params.comment {
        tx.comment = if comment.is_empty() {
            None
        } else {
            Some(comment)
        };
    }

    // Handle account changes.
    if let Some(account_id) = params.account_id {
        let tx_type = classify_transaction(tx);
        match tx_type {
            TransactionType::Expense => {
                tx.outcome_account = AccountId::new(account_id.clone());
                tx.income_account = AccountId::new(account_id.clone());
                let instrument = resolve_instrument(maps, &account_id, None)?;
                tx.outcome_instrument = instrument;
                tx.income_instrument = instrument;
            }
            TransactionType::Income => {
                tx.income_account = AccountId::new(account_id.clone());
                tx.outcome_account = AccountId::new(account_id.clone());
                let instrument = resolve_instrument(maps, &account_id, None)?;
                tx.income_instrument = instrument;
                tx.outcome_instrument = instrument;
            }
            TransactionType::Transfer => {
                tx.outcome_account = AccountId::new(account_id.clone());
                let instrument = resolve_instrument(maps, &account_id, None)?;
                tx.outcome_instrument = instrument;
            }
        }
    }

    if let Some(to_account_id) = params.to_account_id {
        tx.income_account = AccountId::new(to_account_id.clone());
        let instrument = resolve_instrument(maps, &to_account_id, None)?;
        tx.income_instrument = instrument;
    }

    // Handle amount changes.
    if let Some(amount) = params.amount {
        let tx_type = classify_transaction(tx);
        match tx_type {
            TransactionType::Income => tx.income = amount,
            TransactionType::Expense | TransactionType::Transfer => tx.outcome = amount,
        }
    }

    if let Some(to_amount) = params.to_amount {
        tx.income = to_amount;
    }

    tx.changed = Utc::now();

    Ok(())
}

/// Processes bulk operations into push/delete lists without sending to the API.
///
/// Returns `(to_push, to_delete, created_count, updated_count)`.
fn process_bulk_operations(
    operations: Vec<BulkOperation>,
    all_transactions: &[Transaction],
    maps: &LookupMaps,
) -> Result<(Vec<Transaction>, Vec<TransactionId>, usize, usize), McpError> {
    let mut to_push: Vec<Transaction> = Vec::new();
    let mut to_delete: Vec<TransactionId> = Vec::new();
    let mut created_count: usize = 0;
    let mut updated_count: usize = 0;

    for op in operations {
        match op {
            BulkOperation::Create(create_params) => {
                let new_tx = build_transaction(create_params, maps)?;
                to_push.push(new_tx);
                created_count += 1;
            }
            BulkOperation::Update(update_params) => {
                let found = all_transactions
                    .iter()
                    .find(|found_tx| found_tx.id.as_inner() == update_params.id)
                    .ok_or_else(|| {
                        McpError::invalid_params(
                            format!("transaction '{}' not found", update_params.id),
                            None,
                        )
                    })?;
                let mut updated = found.clone();
                apply_update(&mut updated, update_params, maps)?;
                to_push.push(updated);
                updated_count += 1;
            }
            BulkOperation::Delete(delete_params) => {
                if !all_transactions
                    .iter()
                    .any(|found_tx| found_tx.id.as_inner() == delete_params.id)
                {
                    return Err(McpError::invalid_params(
                        format!("transaction '{}' not found", delete_params.id),
                        None,
                    ));
                }
                to_delete.push(TransactionId::new(delete_params.id));
            }
        }
    }

    Ok((to_push, to_delete, created_count, updated_count))
}

#[tool_router]
impl ZenMoneyMcpServer {
    /// Creates a new MCP server with the given ZenMoney client.
    pub(crate) fn new(client: ZenMoney<FileStorage>) -> Self {
        Self {
            client: Arc::new(client),
            tool_router: Self::tool_router(),
            preparations: Arc::new(Mutex::new(HashMap::new())),
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

    /// Lists transactions with optional filtering, sorting, and type/category filters.
    #[tool(
        description = "List transactions with optional filters: date range, account, tag, payee, merchant, amount range, transaction_type (expense/income/transfer), uncategorized (true to show only untagged), sort (asc/desc by date, default desc), and result limit"
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

        // Filter by uncategorized.
        if params.0.uncategorized == Some(true) {
            transactions.retain(is_uncategorized);
        }

        // Filter by transaction type.
        filter_by_transaction_type(&mut transactions, params.0.transaction_type.as_ref());

        // Sort by date.
        let sort_dir = params.0.sort.unwrap_or_default();
        match sort_dir {
            SortDirection::Desc => transactions.sort_by(|left, right| right.date.cmp(&left.date)),
            SortDirection::Asc => transactions.sort_by(|left, right| left.date.cmp(&right.date)),
        }

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
        description = "Suggest a category tag for a transaction based on payee name and/or comment. Note: the ZenMoney API does not provide confidence scores for suggestions"
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

    /// Creates a new transaction with simplified parameters.
    #[tool(
        description = "Create a new financial transaction. Specify transaction_type (expense/income/transfer), date, account_id, and amount. For transfers, also provide to_account_id. Currency instruments are auto-resolved from the account unless overridden with instrument_id/to_instrument_id. Optionally specify tag_ids, payee, and comment"
    )]
    async fn create_transaction(
        &self,
        params: Parameters<CreateTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let new_tx = build_transaction(params.0, &maps)?;
        let response = self
            .client
            .push_transactions(vec![new_tx])
            .await
            .map_err(zen_err)?;

        let result: Vec<TransactionResponse> = response
            .transaction
            .iter()
            .map(|resp_tx| TransactionResponse::from_transaction(resp_tx, &maps))
            .collect();
        json_result(&result)
    }

    /// Updates an existing transaction.
    #[tool(
        description = "Update an existing transaction by ID. All fields except id are optional — only provided fields are changed. Use empty string for payee/comment to clear them. Amount is applied to the correct side (income/outcome) based on the transaction type"
    )]
    async fn update_transaction(
        &self,
        params: Parameters<UpdateTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let all_transactions = self.client.transactions().await.map_err(zen_err)?;
        let mut updated = all_transactions
            .into_iter()
            .find(|found_tx| found_tx.id.as_inner() == params.0.id)
            .ok_or_else(|| {
                McpError::invalid_params(format!("transaction '{}' not found", params.0.id), None)
            })?;

        apply_update(&mut updated, params.0, &maps)?;

        let response = self
            .client
            .push_transactions(vec![updated])
            .await
            .map_err(zen_err)?;

        let result: Vec<TransactionResponse> = response
            .transaction
            .iter()
            .map(|resp_tx| TransactionResponse::from_transaction(resp_tx, &maps))
            .collect();
        json_result(&result)
    }

    /// Deletes a transaction by ID, returning details of the deleted transaction.
    #[tool(
        description = "Delete a transaction by its ID. Returns details of the deleted transaction for confirmation"
    )]
    async fn delete_transaction(
        &self,
        params: Parameters<DeleteTransactionParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;

        // Fetch the transaction details before deleting.
        let all_transactions = self.client.transactions().await.map_err(zen_err)?;
        let existing = all_transactions
            .iter()
            .find(|found_tx| found_tx.id.as_inner() == params.0.id);

        let delete_id = TransactionId::new(params.0.id.clone());
        let _response = self
            .client
            .delete_transactions(&[delete_id])
            .await
            .map_err(zen_err)?;

        if let Some(found_tx) = existing {
            let tx_response = TransactionResponse::from_transaction(found_tx, &maps);
            let result = DeletedTransactionResponse::new(
                format!("Transaction '{}' deleted successfully", params.0.id),
                tx_response,
            );
            json_result(&result)
        } else {
            Ok(CallToolResult::success(vec![Content::text(format!(
                "Transaction '{}' deleted successfully (details not available locally)",
                params.0.id
            ))]))
        }
    }

    /// Validates and prepares bulk operations without executing them.
    ///
    /// Returns a preview with a `preparation_id` that can be passed to
    /// `execute_bulk_operations` to commit the changes.
    #[tool(
        description = "Validate and preview multiple transaction operations (create, update, delete) without executing them. Returns an enriched preview of all changes and a preparation_id. Pass the preparation_id to execute_bulk_operations to commit the changes"
    )]
    async fn prepare_bulk_operations(
        &self,
        params: Parameters<BulkOperationsParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;
        let prepared = self.prepare_operations(params.0.operations, &maps).await?;

        let preparation_id = uuid::Uuid::new_v4().to_string();
        let result = PrepareResponse {
            preparation_id: preparation_id.clone(),
            created: prepared.created_count,
            updated: prepared.updated_count,
            deleted: prepared.to_delete.len(),
            transactions: prepared.preview.clone(),
            deleted_transactions: prepared.deleted_preview.clone(),
        };

        let _prev = self
            .preparations
            .lock()
            .map_err(|err| McpError::internal_error(format!("lock poisoned: {err}"), None))?
            .insert(preparation_id, prepared);

        json_result(&result)
    }

    /// Executes a previously prepared bulk operation.
    ///
    /// Takes the `preparation_id` from `prepare_bulk_operations` and commits
    /// the changes to ZenMoney.
    #[tool(
        description = "Execute a previously prepared bulk operation by its preparation_id (obtained from prepare_bulk_operations). Commits the validated changes to ZenMoney and returns a summary of affected transactions"
    )]
    async fn execute_bulk_operations(
        &self,
        params: Parameters<ExecuteBulkParams>,
    ) -> Result<CallToolResult, McpError> {
        let maps = self.lookup_maps().await?;

        let prepared = self
            .preparations
            .lock()
            .map_err(|err| McpError::internal_error(format!("lock poisoned: {err}"), None))?
            .remove(&params.0.preparation_id)
            .ok_or_else(|| {
                McpError::invalid_params(
                    format!(
                        "preparation '{}' not found or already executed",
                        params.0.preparation_id
                    ),
                    None,
                )
            })?;

        let mut result_transactions: Vec<TransactionResponse> = Vec::new();

        if !prepared.to_push.is_empty() {
            let response = self
                .client
                .push_transactions(prepared.to_push)
                .await
                .map_err(zen_err)?;
            result_transactions.extend(
                response
                    .transaction
                    .iter()
                    .map(|resp_tx| TransactionResponse::from_transaction(resp_tx, &maps)),
            );
        }

        let deleted_count = prepared.to_delete.len();
        if !prepared.to_delete.is_empty() {
            let _response = self
                .client
                .delete_transactions(&prepared.to_delete)
                .await
                .map_err(zen_err)?;
        }

        let result = BulkOperationsResponse::new(
            prepared.created_count,
            prepared.updated_count,
            deleted_count,
            result_transactions,
        );
        json_result(&result)
    }

    /// Validates all operations, resolves instruments, and builds a [`PreparedBulk`].
    ///
    /// No data is sent to ZenMoney — this is the dry-run step.
    async fn prepare_operations(
        &self,
        operations: Vec<BulkOperation>,
        maps: &LookupMaps,
    ) -> Result<PreparedBulk, McpError> {
        let all_transactions = self.client.transactions().await.map_err(zen_err)?;
        let (to_push, to_delete, created_count, updated_count) =
            process_bulk_operations(operations, &all_transactions, maps)?;

        let preview: Vec<TransactionResponse> = to_push
            .iter()
            .map(|tx| TransactionResponse::from_transaction(tx, maps))
            .collect();
        let deleted_preview: Vec<TransactionResponse> = to_delete
            .iter()
            .filter_map(|del_id| {
                all_transactions
                    .iter()
                    .find(|tx| tx.id.as_inner() == del_id.as_inner())
            })
            .map(|tx| TransactionResponse::from_transaction(tx, maps))
            .collect();

        Ok(PreparedBulk {
            to_push,
            to_delete,
            created_count,
            updated_count,
            preview,
            deleted_preview,
        })
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
