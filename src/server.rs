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
    AccountId, InstrumentId, MerchantId, NaiveDate, SuggestRequest, Tag, TagId, Transaction,
    TransactionId, UserId,
};
#[cfg(test)]
use zenmoney_rs::storage::InMemoryStorage;
use zenmoney_rs::storage::{FileStorage, Storage};
use zenmoney_rs::zen_money::{TransactionFilter, ZenMoney};

use chrono::{DateTime, Utc};

use crate::params::{
    BulkOperation, BulkOperationsParams, CreateTagParams, CreateTransactionParams,
    DeleteTransactionParams, ExecuteBulkParams, FindAccountParams, FindTagParams,
    GetInstrumentParams, ListAccountsParams, ListBudgetsParams, ListTransactionsParams,
    SortDirection, SuggestCategoryParams, TransactionType, UpdateTransactionParams,
};
use crate::response::{
    AccountResponse, BudgetResponse, BulkOperationsResponse, DeletedTransactionResponse,
    InstrumentResponse, LookupMaps, MerchantResponse, PrepareResponse, ReminderResponse,
    SuggestResponse, TagResponse, TransactionResponse, build_lookup_maps,
};

/// Maximum number of operations allowed in a single bulk call.
const MAX_BULK_OPERATIONS: usize = 20;

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
}

/// MCP server wrapping the ZenMoney personal finance API.
#[derive(Clone)]
pub(crate) struct ZenMoneyMcpServer<S: Storage + 'static = FileStorage> {
    /// Inner ZenMoney client (shared via Arc).
    client: Arc<ZenMoney<S>>,
    /// Tool router for dispatching MCP tool calls.
    tool_router: ToolRouter<Self>,
    /// In-memory store of prepared bulk operations awaiting execution.
    preparations: Arc<Mutex<HashMap<String, PreparedBulk>>>,
}

impl<S: Storage + 'static> core::fmt::Debug for ZenMoneyMcpServer<S> {
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

/// Validates and normalizes a tag title.
///
/// Trims leading/trailing whitespace and rejects empty/blank titles.
fn normalize_tag_title(title: &str) -> Result<String, McpError> {
    let trimmed = title.trim();
    if trimmed.is_empty() {
        return Err(McpError::invalid_params(
            "title must not be empty or blank".to_owned(),
            None,
        ));
    }
    Ok(trimmed.to_owned())
}

/// Normalizes text for case-insensitive tag title comparison.
fn normalized_title_key(title: &str) -> String {
    title.trim().to_lowercase()
}

/// Finds an existing tag by title using case-insensitive matching.
fn find_tag_by_title_case_insensitive<'tag>(tags: &'tag [Tag], title: &str) -> Option<&'tag Tag> {
    let key = normalized_title_key(title);
    tags.iter()
        .find(|tag| normalized_title_key(&tag.title) == key)
}

/// Validates that `parent_tag_id` exists in the current tag list.
fn validate_parent_tag_exists(tags: &[Tag], parent_tag_id: Option<&str>) -> Result<(), McpError> {
    if let Some(parent_id) = parent_tag_id {
        let parent_exists = tags.iter().any(|tag| tag.id.as_inner() == parent_id);
        if !parent_exists {
            return Err(McpError::invalid_params(
                format!("parent_tag_id '{parent_id}' not found"),
                None,
            ));
        }
    }
    Ok(())
}

/// Builds a new [`Tag`] from validated creation parameters.
fn build_tag(params: CreateTagParams, user_id: i64, title: String) -> Tag {
    Tag {
        id: TagId::new(uuid::Uuid::new_v4().to_string()),
        changed: Utc::now(),
        user: UserId::new(user_id),
        title,
        parent: params.parent_tag_id.map(TagId::new),
        icon: params.icon,
        picture: None,
        color: params.color,
        show_income: params.show_income.unwrap_or(false),
        show_outcome: params.show_outcome.unwrap_or(true),
        budget_income: params.budget_income.unwrap_or(false),
        budget_outcome: params.budget_outcome.unwrap_or(true),
        required: params.required,
        static_id: None,
        archive: Some(false),
    }
}

#[tool_router]
impl<S: Storage + 'static> ZenMoneyMcpServer<S> {
    /// Creates a new MCP server with the given ZenMoney client.
    pub(crate) fn new(client: ZenMoney<S>) -> Self {
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

    /// Returns the first synced user ID, or `0` when local storage has no users.
    async fn current_user_id(&self) -> Result<i64, McpError> {
        let users = self.client.users().await.map_err(zen_err)?;
        Ok(users.first().map_or(0, |user| user.id.into_inner()))
    }

    /// Shared implementation for `create_tag` and `create_category`.
    async fn create_tag_internal(
        &self,
        params: CreateTagParams,
    ) -> Result<CallToolResult, McpError> {
        let normalized_title = normalize_tag_title(&params.title)?;
        let tags = self.client.tags().await.map_err(zen_err)?;

        if let Some(existing_tag) = find_tag_by_title_case_insensitive(&tags, &normalized_title) {
            let maps = self.lookup_maps().await?;
            let result = TagResponse::from_tag(existing_tag, &maps);
            return json_result(&result);
        }

        validate_parent_tag_exists(&tags, params.parent_tag_id.as_deref())?;

        let user_id = self.current_user_id().await?;
        let new_tag = build_tag(params, user_id, normalized_title);
        let maps = self.lookup_maps().await?;
        let preview = TagResponse::from_tag(&new_tag, &maps);

        let _response = self
            .client
            .push_tags(vec![new_tag])
            .await
            .map_err(zen_err)?;

        json_result(&preview)
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
        let preview = TransactionResponse::from_transaction(&new_tx, &maps);
        let _response = self
            .client
            .push_transactions(vec![new_tx])
            .await
            .map_err(zen_err)?;

        json_result(&vec![preview])
    }

    /// Creates a new category tag.
    #[tool(
        description = "Create a new category tag. If a tag with the same title already exists (case-insensitive), returns the existing tag instead of creating a duplicate"
    )]
    async fn create_tag(
        &self,
        params: Parameters<CreateTagParams>,
    ) -> Result<CallToolResult, McpError> {
        self.create_tag_internal(params.0).await
    }

    /// Alias for creating a category tag.
    #[tool(
        description = "Alias for create_tag: create a category tag with the same behavior and idempotency guarantees"
    )]
    async fn create_category(
        &self,
        params: Parameters<CreateTagParams>,
    ) -> Result<CallToolResult, McpError> {
        self.create_tag_internal(params.0).await
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

        let preview = TransactionResponse::from_transaction(&updated, &maps);
        let _response = self
            .client
            .push_transactions(vec![updated])
            .await
            .map_err(zen_err)?;

        json_result(&vec![preview])
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
        description = "Validate and preview multiple transaction operations (create, update, delete) without executing them. Returns an enriched preview of all changes and a preparation_id. Pass the preparation_id to execute_bulk_operations to commit the changes. IMPORTANT: limit to 10 operations per call to avoid transport timeouts; split larger batches into multiple prepare calls"
    )]
    async fn prepare_bulk_operations(
        &self,
        params: Parameters<BulkOperationsParams>,
    ) -> Result<CallToolResult, McpError> {
        tracing::debug!("prepare_bulk_operations: start");

        if params.0.operations.len() > MAX_BULK_OPERATIONS {
            return Err(McpError::invalid_params(
                format!(
                    "too many operations ({}); limit is {MAX_BULK_OPERATIONS} per call — split into smaller batches",
                    params.0.operations.len()
                ),
                None,
            ));
        }

        let maps = self.lookup_maps().await?;
        tracing::debug!("prepare_bulk_operations: lookup_maps done");

        let all_transactions = self.client.transactions().await.map_err(zen_err)?;
        tracing::debug!(
            count = all_transactions.len(),
            "prepare_bulk_operations: loaded transactions"
        );

        let (to_push, to_delete, created_count, updated_count) =
            process_bulk_operations(params.0.operations, &all_transactions, &maps)?;
        tracing::debug!(
            created_count,
            updated_count,
            deleted = to_delete.len(),
            "prepare_bulk_operations: processed operations"
        );

        let preview: Vec<TransactionResponse> = to_push
            .iter()
            .map(|tx| TransactionResponse::from_transaction(tx, &maps))
            .collect();
        let deleted_preview: Vec<TransactionResponse> = to_delete
            .iter()
            .filter_map(|del_id| {
                all_transactions
                    .iter()
                    .find(|tx| tx.id.as_inner() == del_id.as_inner())
            })
            .map(|tx| TransactionResponse::from_transaction(tx, &maps))
            .collect();

        let preparation_id = uuid::Uuid::new_v4().to_string();
        let result = PrepareResponse {
            preparation_id: preparation_id.clone(),
            created: created_count,
            updated: updated_count,
            deleted: to_delete.len(),
            transactions: preview,
            deleted_transactions: deleted_preview,
        };

        let prepared = PreparedBulk {
            to_push,
            to_delete,
            created_count,
            updated_count,
        };

        let _prev = self
            .preparations
            .lock()
            .map_err(|err| McpError::internal_error(format!("lock poisoned: {err}"), None))?
            .insert(preparation_id, prepared);

        tracing::debug!("prepare_bulk_operations: done");
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

        // Build previews from local data before consuming prepared transactions.
        let push_preview: Vec<TransactionResponse> = prepared
            .to_push
            .iter()
            .map(|tx| TransactionResponse::from_transaction(tx, &maps))
            .collect();

        if !prepared.to_push.is_empty() {
            let _response = self
                .client
                .push_transactions(prepared.to_push)
                .await
                .map_err(zen_err)?;
        }

        // Look up deleted transactions before deleting.
        let mut deleted_preview: Vec<TransactionResponse> = Vec::new();
        let deleted_count = prepared.to_delete.len();
        if !prepared.to_delete.is_empty() {
            let all_transactions = self.client.transactions().await.map_err(zen_err)?;
            deleted_preview = prepared
                .to_delete
                .iter()
                .filter_map(|del_id| {
                    all_transactions
                        .iter()
                        .find(|tx| tx.id.as_inner() == del_id.as_inner())
                })
                .map(|tx| TransactionResponse::from_transaction(tx, &maps))
                .collect();

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
            push_preview,
            deleted_preview,
        );
        json_result(&result)
    }
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::shadow_reuse,
    clippy::missing_docs_in_private_items,
    reason = "test code uses expect and shadow reuse for readability"
)]
mod tests {
    use super::*;
    use chrono::DateTime;

    fn test_timestamp() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test")
    }

    fn test_date() -> NaiveDate {
        NaiveDate::from_ymd_opt(2024, 6, 15).expect("valid date for test")
    }

    fn sample_maps() -> LookupMaps {
        use zenmoney_rs::models::{Account, AccountType, Instrument, Tag};

        let accounts = vec![
            Account {
                id: AccountId::new("acc-1".to_owned()),
                changed: test_timestamp(),
                user: UserId::new(1),
                role: None,
                instrument: Some(InstrumentId::new(1)),
                company: None,
                kind: AccountType::Checking,
                title: "Main Account".to_owned(),
                sync_id: None,
                balance: Some(50_000.0),
                start_balance: None,
                credit_limit: None,
                in_balance: true,
                savings: None,
                enable_correction: false,
                enable_sms: false,
                archive: false,
                capitalization: None,
                percent: None,
                start_date: None,
                end_date_offset: None,
                end_date_offset_interval: None,
                payoff_step: None,
                payoff_interval: None,
                balance_correction_type: None,
                private: None,
            },
            Account {
                id: AccountId::new("acc-2".to_owned()),
                changed: test_timestamp(),
                user: UserId::new(1),
                role: None,
                instrument: Some(InstrumentId::new(2)),
                company: None,
                kind: AccountType::Cash,
                title: "USD Account".to_owned(),
                sync_id: None,
                balance: Some(1_000.0),
                start_balance: None,
                credit_limit: None,
                in_balance: true,
                savings: None,
                enable_correction: false,
                enable_sms: false,
                archive: false,
                capitalization: None,
                percent: None,
                start_date: None,
                end_date_offset: None,
                end_date_offset_interval: None,
                payoff_step: None,
                payoff_interval: None,
                balance_correction_type: None,
                private: None,
            },
        ];
        let tags = vec![Tag {
            id: TagId::new("tag-1".to_owned()),
            changed: test_timestamp(),
            user: UserId::new(1),
            title: "Groceries".to_owned(),
            parent: None,
            icon: None,
            picture: None,
            color: None,
            show_income: false,
            show_outcome: true,
            budget_income: false,
            budget_outcome: true,
            required: None,
            static_id: None,
            archive: None,
        }];
        let instruments = vec![
            Instrument {
                id: InstrumentId::new(1),
                changed: test_timestamp(),
                title: "Russian Ruble".to_owned(),
                short_title: "RUB".to_owned(),
                symbol: "\u{20bd}".to_owned(),
                rate: 1.0,
            },
            Instrument {
                id: InstrumentId::new(2),
                changed: test_timestamp(),
                title: "US Dollar".to_owned(),
                short_title: "USD".to_owned(),
                symbol: "$".to_owned(),
                rate: 90.0,
            },
        ];
        build_lookup_maps(&accounts, &tags, &instruments)
    }

    fn sample_transaction(id: &str, outcome: f64, income: f64) -> Transaction {
        Transaction {
            id: TransactionId::new(id.to_owned()),
            changed: test_timestamp(),
            created: test_timestamp(),
            user: UserId::new(1),
            deleted: false,
            hold: None,
            income_instrument: InstrumentId::new(1),
            income_account: AccountId::new("acc-1".to_owned()),
            income,
            outcome_instrument: InstrumentId::new(1),
            outcome_account: AccountId::new("acc-1".to_owned()),
            outcome,
            tag: None,
            merchant: None,
            payee: None,
            original_payee: None,
            comment: None,
            date: test_date(),
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
        }
    }

    fn sample_transfer(id: &str, outcome: f64, income: f64) -> Transaction {
        let mut tx = sample_transaction(id, outcome, income);
        tx.outcome_account = AccountId::new("acc-1".to_owned());
        tx.income_account = AccountId::new("acc-2".to_owned());
        tx.income_instrument = InstrumentId::new(2);
        tx
    }

    fn sample_create_params(tx_type: TransactionType) -> CreateTransactionParams {
        CreateTransactionParams {
            transaction_type: tx_type,
            date: "2024-06-15".to_owned(),
            account_id: "acc-1".to_owned(),
            amount: 500.0,
            to_account_id: None,
            to_amount: None,
            instrument_id: None,
            to_instrument_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        }
    }

    fn sample_create_tag_params(title: &str) -> CreateTagParams {
        CreateTagParams {
            title: title.to_owned(),
            parent_tag_id: None,
            icon: None,
            color: None,
            show_income: None,
            show_outcome: None,
            budget_income: None,
            budget_outcome: None,
            required: None,
        }
    }

    // ── parse_date ──────────────────────────────────────────────────

    #[test]
    fn parse_date_valid() {
        let date = parse_date("2024-06-15").expect("valid date");
        assert_eq!(date, NaiveDate::from_ymd_opt(2024, 6, 15).expect("valid"));
    }

    #[test]
    fn parse_date_invalid_format() {
        let result = parse_date("15-06-2024");
        assert!(result.is_err());
    }

    #[test]
    fn parse_date_invalid_date() {
        let result = parse_date("2024-13-40");
        assert!(result.is_err());
    }

    // ── tag helpers ────────────────────────────────────────────────

    #[test]
    fn normalize_tag_title_trims_text() {
        let normalized = normalize_tag_title("  Rent an apartment  ").expect("valid title");
        assert_eq!(normalized, "Rent an apartment");
    }

    #[test]
    fn normalize_tag_title_blank_errors() {
        let result = normalize_tag_title("   ");
        assert!(result.is_err());
    }

    #[test]
    fn find_tag_by_title_case_insensitive_matches_existing() {
        let tags = vec![Tag {
            id: TagId::new("tag-1".to_owned()),
            changed: test_timestamp(),
            user: UserId::new(1),
            title: "Groceries".to_owned(),
            parent: None,
            icon: None,
            picture: None,
            color: None,
            show_income: false,
            show_outcome: true,
            budget_income: false,
            budget_outcome: true,
            required: None,
            static_id: None,
            archive: None,
        }];
        let key = "gRoCeRiEs";
        let tag = find_tag_by_title_case_insensitive(&tags, key);
        assert!(tag.is_some());
    }

    #[test]
    fn build_tag_uses_expense_defaults() {
        let params = sample_create_tag_params("Utilities");
        let tag = build_tag(params, 5, "Utilities".to_owned());
        assert_eq!(tag.title, "Utilities");
        assert_eq!(tag.user, UserId::new(5));
        assert!(!tag.show_income);
        assert!(tag.show_outcome);
        assert!(!tag.budget_income);
        assert!(tag.budget_outcome);
        assert_eq!(tag.archive, Some(false));
    }

    // ── to_json_text / json_result ──────────────────────────────────

    #[test]
    fn to_json_text_serializes_pretty() {
        #[derive(serde::Serialize)]
        struct Simple {
            name: String,
        }
        let val = Simple {
            name: "test".to_owned(),
        };
        let text = to_json_text(&val).expect("should serialize");
        assert!(text.contains("\"name\": \"test\""));
        // Pretty-printed means it has newlines.
        assert!(text.contains('\n'));
    }

    #[test]
    fn json_result_returns_call_tool_result() {
        let val = vec![1, 2, 3];
        let result = json_result(&val).expect("should produce result");
        assert!(!result.is_error.unwrap_or(false));
        assert!(!result.content.is_empty());
    }

    // ── account_type_label ──────────────────────────────────────────

    #[test]
    fn account_type_label_all_variants() {
        use zenmoney_rs::models::AccountType;
        assert_eq!(account_type_label(AccountType::Cash), "Cash");
        assert_eq!(account_type_label(AccountType::CreditCard), "CreditCard");
        assert_eq!(account_type_label(AccountType::Checking), "Checking");
        assert_eq!(account_type_label(AccountType::Loan), "Loan");
        assert_eq!(account_type_label(AccountType::Deposit), "Deposit");
        assert_eq!(account_type_label(AccountType::EMoney), "EMoney");
        assert_eq!(account_type_label(AccountType::Debt), "Debt");
    }

    // ── resolve_instrument ──────────────────────────────────────────

    #[test]
    fn resolve_instrument_explicit_overrides() {
        let maps = sample_maps();
        let result = resolve_instrument(&maps, "acc-1", Some(42)).expect("should resolve");
        assert_eq!(result.into_inner(), 42);
    }

    #[test]
    fn resolve_instrument_from_maps() {
        let maps = sample_maps();
        let result = resolve_instrument(&maps, "acc-1", None).expect("should resolve");
        assert_eq!(result.into_inner(), 1);
    }

    #[test]
    fn resolve_instrument_unknown_account_errors() {
        let maps = sample_maps();
        let result = resolve_instrument(&maps, "unknown", None);
        assert!(result.is_err());
    }

    // ── classify_transaction ────────────────────────────────────────

    #[test]
    fn classify_expense() {
        let tx = sample_transaction("tx-1", 500.0, 0.0);
        assert!(matches!(
            classify_transaction(&tx),
            TransactionType::Expense
        ));
    }

    #[test]
    fn classify_income() {
        let tx = sample_transaction("tx-1", 0.0, 1000.0);
        assert!(matches!(classify_transaction(&tx), TransactionType::Income));
    }

    #[test]
    fn classify_transfer() {
        let tx = sample_transfer("tx-1", 500.0, 500.0);
        assert!(matches!(
            classify_transaction(&tx),
            TransactionType::Transfer
        ));
    }

    #[test]
    fn classify_same_account_both_positive_is_income() {
        // Both positive but same account → Income (not Transfer).
        let tx = sample_transaction("tx-1", 100.0, 200.0);
        assert!(matches!(classify_transaction(&tx), TransactionType::Income));
    }

    // ── filter_by_transaction_type ──────────────────────────────────

    #[test]
    fn filter_expense_retains_only_expenses() {
        let mut txs = vec![
            sample_transaction("tx-1", 500.0, 0.0),  // expense
            sample_transaction("tx-2", 0.0, 1000.0), // income
            sample_transfer("tx-3", 300.0, 300.0),   // transfer
        ];
        filter_by_transaction_type(&mut txs, Some(&TransactionType::Expense));
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id.as_inner(), "tx-1");
    }

    #[test]
    fn filter_income_retains_only_income() {
        let mut txs = vec![
            sample_transaction("tx-1", 500.0, 0.0),
            sample_transaction("tx-2", 0.0, 1000.0),
        ];
        filter_by_transaction_type(&mut txs, Some(&TransactionType::Income));
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id.as_inner(), "tx-2");
    }

    #[test]
    fn filter_transfer_retains_only_transfers() {
        let mut txs = vec![
            sample_transaction("tx-1", 500.0, 0.0),
            sample_transfer("tx-2", 300.0, 300.0),
        ];
        filter_by_transaction_type(&mut txs, Some(&TransactionType::Transfer));
        assert_eq!(txs.len(), 1);
        assert_eq!(txs[0].id.as_inner(), "tx-2");
    }

    #[test]
    fn filter_none_keeps_all() {
        let mut txs = vec![
            sample_transaction("tx-1", 500.0, 0.0),
            sample_transaction("tx-2", 0.0, 1000.0),
        ];
        filter_by_transaction_type(&mut txs, None);
        assert_eq!(txs.len(), 2);
    }

    // ── is_uncategorized ────────────────────────────────────────────

    #[test]
    fn is_uncategorized_no_tags() {
        let tx = sample_transaction("tx-1", 500.0, 0.0);
        assert!(is_uncategorized(&tx));
    }

    #[test]
    fn is_uncategorized_empty_vec() {
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        tx.tag = Some(vec![]);
        assert!(is_uncategorized(&tx));
    }

    #[test]
    fn is_uncategorized_with_tags() {
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        tx.tag = Some(vec![TagId::new("tag-1".to_owned())]);
        assert!(!is_uncategorized(&tx));
    }

    // ── resolve_sides ───────────────────────────────────────────────

    #[test]
    fn resolve_sides_expense() {
        let maps = sample_maps();
        let params = sample_create_params(TransactionType::Expense);
        let sides = resolve_sides(&params, &maps).expect("should resolve");
        assert!((sides.outcome - 500.0).abs() < f64::EPSILON);
        assert!((sides.income - 0.0).abs() < f64::EPSILON);
        assert_eq!(sides.outcome_account.as_inner(), "acc-1");
    }

    #[test]
    fn resolve_sides_income() {
        let maps = sample_maps();
        let params = sample_create_params(TransactionType::Income);
        let sides = resolve_sides(&params, &maps).expect("should resolve");
        assert!((sides.income - 500.0).abs() < f64::EPSILON);
        assert!((sides.outcome - 0.0).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_sides_transfer() {
        let maps = sample_maps();
        let mut params = sample_create_params(TransactionType::Transfer);
        params.to_account_id = Some("acc-2".to_owned());
        params.to_amount = Some(7.0);
        let sides = resolve_sides(&params, &maps).expect("should resolve");
        assert!((sides.outcome - 500.0).abs() < f64::EPSILON);
        assert!((sides.income - 7.0).abs() < f64::EPSILON);
        assert_eq!(sides.income_account.as_inner(), "acc-2");
        assert_eq!(sides.income_instrument.into_inner(), 2);
    }

    #[test]
    fn resolve_sides_transfer_defaults_to_amount() {
        let maps = sample_maps();
        let mut params = sample_create_params(TransactionType::Transfer);
        params.to_account_id = Some("acc-2".to_owned());
        // No to_amount — should default to amount.
        let sides = resolve_sides(&params, &maps).expect("should resolve");
        assert!((sides.income - 500.0).abs() < f64::EPSILON);
    }

    #[test]
    fn resolve_sides_transfer_missing_to_account_errors() {
        let maps = sample_maps();
        let params = sample_create_params(TransactionType::Transfer);
        let result = resolve_sides(&params, &maps);
        assert!(result.is_err());
    }

    // ── build_transaction ───────────────────────────────────────────

    #[test]
    fn build_transaction_expense_with_optional_fields() {
        let maps = sample_maps();
        let mut params = sample_create_params(TransactionType::Expense);
        params.tag_ids = Some(vec!["tag-1".to_owned()]);
        params.payee = Some("Coffee Shop".to_owned());
        params.comment = Some("Morning coffee".to_owned());

        let tx = build_transaction(params, &maps).expect("should build");
        assert!((tx.outcome - 500.0).abs() < f64::EPSILON);
        assert!((tx.income - 0.0).abs() < f64::EPSILON);
        assert_eq!(tx.tag.as_ref().expect("should have tags").len(), 1);
        assert_eq!(tx.payee.as_deref(), Some("Coffee Shop"));
        assert_eq!(tx.comment.as_deref(), Some("Morning coffee"));
        assert_eq!(tx.date, test_date());
    }

    #[test]
    fn build_transaction_income_minimal() {
        let maps = sample_maps();
        let params = sample_create_params(TransactionType::Income);
        let tx = build_transaction(params, &maps).expect("should build");
        assert!((tx.income - 500.0).abs() < f64::EPSILON);
        assert!((tx.outcome - 0.0).abs() < f64::EPSILON);
        assert!(tx.tag.is_none());
        assert!(tx.payee.is_none());
    }

    #[test]
    fn build_transaction_invalid_date_errors() {
        let maps = sample_maps();
        let mut params = sample_create_params(TransactionType::Expense);
        params.date = "not-a-date".to_owned();
        let result = build_transaction(params, &maps);
        assert!(result.is_err());
    }

    // ── apply_update ────────────────────────────────────────────────

    #[test]
    fn apply_update_date() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: Some("2025-01-01".to_owned()),
            amount: None,
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert_eq!(tx.date, NaiveDate::from_ymd_opt(2025, 1, 1).expect("valid"));
    }

    #[test]
    fn apply_update_payee_empty_clears() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        tx.payee = Some("Old Payee".to_owned());
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: Some(String::new()),
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert!(tx.payee.is_none());
    }

    #[test]
    fn apply_update_comment_empty_clears() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        tx.comment = Some("Old comment".to_owned());
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: Some(String::new()),
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert!(tx.comment.is_none());
    }

    #[test]
    fn apply_update_tag_ids() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: Some(vec!["tag-1".to_owned(), "tag-2".to_owned()]),
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        let tags = tx.tag.expect("should have tags");
        assert_eq!(tags.len(), 2);
    }

    #[test]
    fn apply_update_amount_on_expense() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: Some(750.0),
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert!((tx.outcome - 750.0).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_update_account_on_transfer() {
        let maps = sample_maps();
        let mut tx = sample_transfer("tx-1", 500.0, 500.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: Some("acc-2".to_owned()),
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert_eq!(tx.outcome_account.as_inner(), "acc-2");
        assert_eq!(tx.outcome_instrument.into_inner(), 2);
    }

    #[test]
    fn apply_update_comment_sets_value() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: Some("New comment".to_owned()),
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert_eq!(tx.comment.as_deref(), Some("New comment"));
    }

    #[test]
    fn apply_update_account_on_expense() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 500.0, 0.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: Some("acc-2".to_owned()),
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert_eq!(tx.outcome_account.as_inner(), "acc-2");
        assert_eq!(tx.income_account.as_inner(), "acc-2");
        assert_eq!(tx.outcome_instrument.into_inner(), 2);
        assert_eq!(tx.income_instrument.into_inner(), 2);
    }

    #[test]
    fn apply_update_account_on_income() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 0.0, 1000.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: Some("acc-2".to_owned()),
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert_eq!(tx.income_account.as_inner(), "acc-2");
        assert_eq!(tx.outcome_account.as_inner(), "acc-2");
        assert_eq!(tx.income_instrument.into_inner(), 2);
        assert_eq!(tx.outcome_instrument.into_inner(), 2);
    }

    #[test]
    fn apply_update_to_account_id() {
        let maps = sample_maps();
        let mut tx = sample_transfer("tx-1", 500.0, 500.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: None,
            account_id: None,
            to_account_id: Some("acc-1".to_owned()),
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert_eq!(tx.income_account.as_inner(), "acc-1");
        assert_eq!(tx.income_instrument.into_inner(), 1);
    }

    #[test]
    fn apply_update_amount_on_income() {
        let maps = sample_maps();
        let mut tx = sample_transaction("tx-1", 0.0, 1000.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: Some(2000.0),
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert!((tx.income - 2000.0).abs() < f64::EPSILON);
    }

    #[test]
    fn apply_update_to_amount() {
        let maps = sample_maps();
        let mut tx = sample_transfer("tx-1", 500.0, 500.0);
        let params = UpdateTransactionParams {
            id: "tx-1".to_owned(),
            date: None,
            amount: None,
            to_amount: Some(750.0),
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        };
        apply_update(&mut tx, params, &maps).expect("should update");
        assert!((tx.income - 750.0).abs() < f64::EPSILON);
    }

    // ── process_bulk_operations ─────────────────────────────────────

    #[test]
    fn process_bulk_create_update_delete_mix() {
        let maps = sample_maps();
        let existing = vec![sample_transaction("tx-existing", 100.0, 0.0)];
        let operations = vec![
            BulkOperation::Create(sample_create_params(TransactionType::Expense)),
            BulkOperation::Update(UpdateTransactionParams {
                id: "tx-existing".to_owned(),
                date: None,
                amount: Some(200.0),
                to_amount: None,
                account_id: None,
                to_account_id: None,
                tag_ids: None,
                payee: None,
                comment: None,
            }),
            BulkOperation::Delete(DeleteTransactionParams {
                id: "tx-existing".to_owned(),
            }),
        ];
        let (to_push, to_delete, created, updated) =
            process_bulk_operations(operations, &existing, &maps).expect("should process");
        assert_eq!(created, 1);
        assert_eq!(updated, 1);
        assert_eq!(to_push.len(), 2);
        assert_eq!(to_delete.len(), 1);
    }

    #[test]
    fn process_bulk_update_nonexistent_errors() {
        let maps = sample_maps();
        let existing: Vec<Transaction> = vec![];
        let operations = vec![BulkOperation::Update(UpdateTransactionParams {
            id: "no-such-tx".to_owned(),
            date: None,
            amount: Some(100.0),
            to_amount: None,
            account_id: None,
            to_account_id: None,
            tag_ids: None,
            payee: None,
            comment: None,
        })];
        let result = process_bulk_operations(operations, &existing, &maps);
        assert!(result.is_err());
    }

    #[test]
    fn process_bulk_delete_nonexistent_errors() {
        let maps = sample_maps();
        let existing: Vec<Transaction> = vec![];
        let operations = vec![BulkOperation::Delete(DeleteTransactionParams {
            id: "no-such-tx".to_owned(),
        })];
        let result = process_bulk_operations(operations, &existing, &maps);
        assert!(result.is_err());
    }

    #[test]
    fn process_bulk_empty_operations() {
        let maps = sample_maps();
        let existing: Vec<Transaction> = vec![];
        let (to_push, to_delete, created, updated) =
            process_bulk_operations(vec![], &existing, &maps).expect("should process");
        assert!(to_push.is_empty());
        assert!(to_delete.is_empty());
        assert_eq!(created, 0);
        assert_eq!(updated, 0);
    }

    #[test]
    fn process_bulk_all_deletes() {
        let maps = sample_maps();
        let existing = vec![
            sample_transaction("tx-1", 100.0, 0.0),
            sample_transaction("tx-2", 200.0, 0.0),
        ];
        let operations = vec![
            BulkOperation::Delete(DeleteTransactionParams {
                id: "tx-1".to_owned(),
            }),
            BulkOperation::Delete(DeleteTransactionParams {
                id: "tx-2".to_owned(),
            }),
        ];
        let (to_push, to_delete, created, updated) =
            process_bulk_operations(operations, &existing, &maps).expect("should process");
        assert!(to_push.is_empty());
        assert_eq!(to_delete.len(), 2);
        assert_eq!(created, 0);
        assert_eq!(updated, 0);
    }

    // ── Async handler tests (using InMemoryStorage) ─────────────────

    async fn build_test_server() -> ZenMoneyMcpServer<InMemoryStorage> {
        use zenmoney_rs::models::{
            Account, AccountType, Budget, Instrument, Merchant, Reminder, ReminderId, Tag,
        };

        let storage = InMemoryStorage::new();
        let client = ZenMoney::builder()
            .token("test-token")
            .storage(storage)
            .build()
            .expect("should build test client");
        let accounts = vec![
            Account {
                id: AccountId::new("acc-1".to_owned()),
                changed: test_timestamp(),
                user: UserId::new(1),
                role: None,
                instrument: Some(InstrumentId::new(1)),
                company: None,
                kind: AccountType::Checking,
                title: "Main Account".to_owned(),
                sync_id: None,
                balance: Some(50_000.0),
                start_balance: None,
                credit_limit: None,
                in_balance: true,
                savings: None,
                enable_correction: false,
                enable_sms: false,
                archive: false,
                capitalization: None,
                percent: None,
                start_date: None,
                end_date_offset: None,
                end_date_offset_interval: None,
                payoff_step: None,
                payoff_interval: None,
                balance_correction_type: None,
                private: None,
            },
            Account {
                id: AccountId::new("acc-2".to_owned()),
                changed: test_timestamp(),
                user: UserId::new(1),
                role: None,
                instrument: Some(InstrumentId::new(2)),
                company: None,
                kind: AccountType::Cash,
                title: "USD Account".to_owned(),
                sync_id: None,
                balance: Some(1_000.0),
                start_balance: None,
                credit_limit: None,
                in_balance: true,
                savings: None,
                enable_correction: false,
                enable_sms: false,
                archive: true,
                capitalization: None,
                percent: None,
                start_date: None,
                end_date_offset: None,
                end_date_offset_interval: None,
                payoff_step: None,
                payoff_interval: None,
                balance_correction_type: None,
                private: None,
            },
        ];
        let tags = vec![Tag {
            id: TagId::new("tag-1".to_owned()),
            changed: test_timestamp(),
            user: UserId::new(1),
            title: "Groceries".to_owned(),
            parent: None,
            icon: None,
            picture: None,
            color: None,
            show_income: false,
            show_outcome: true,
            budget_income: false,
            budget_outcome: true,
            required: None,
            static_id: None,
            archive: None,
        }];
        let instruments = vec![
            Instrument {
                id: InstrumentId::new(1),
                changed: test_timestamp(),
                title: "Russian Ruble".to_owned(),
                short_title: "RUB".to_owned(),
                symbol: "\u{20bd}".to_owned(),
                rate: 1.0,
            },
            Instrument {
                id: InstrumentId::new(2),
                changed: test_timestamp(),
                title: "US Dollar".to_owned(),
                short_title: "USD".to_owned(),
                symbol: "$".to_owned(),
                rate: 90.0,
            },
        ];
        let transactions = vec![
            sample_transaction("tx-expense", 500.0, 0.0),
            sample_transaction("tx-income", 0.0, 1000.0),
            sample_transfer("tx-transfer", 300.0, 300.0),
        ];
        let merchants = vec![Merchant {
            id: MerchantId::new("m-1".to_owned()),
            changed: test_timestamp(),
            user: UserId::new(1),
            title: "Coffee Shop".to_owned(),
        }];
        let budgets = vec![Budget {
            changed: test_timestamp(),
            user: UserId::new(1),
            tag: Some(TagId::new("tag-1".to_owned())),
            date: NaiveDate::from_ymd_opt(2024, 6, 1).expect("valid date"),
            income: 0.0,
            income_lock: false,
            outcome: 15_000.0,
            outcome_lock: false,
            is_income_forecast: None,
            is_outcome_forecast: None,
        }];
        let reminders = vec![Reminder {
            id: ReminderId::new("rem-1".to_owned()),
            changed: test_timestamp(),
            user: UserId::new(1),
            income_instrument: InstrumentId::new(1),
            income_account: AccountId::new("acc-1".to_owned()),
            income: 0.0,
            outcome_instrument: InstrumentId::new(1),
            outcome_account: AccountId::new("acc-1".to_owned()),
            outcome: 5_000.0,
            tag: Some(vec![TagId::new("tag-1".to_owned())]),
            merchant: None,
            payee: Some("Supermarket".to_owned()),
            comment: None,
            interval: None,
            step: None,
            points: None,
            start_date: test_date(),
            end_date: None,
            notify: false,
        }];

        client
            .storage()
            .upsert_accounts(accounts)
            .await
            .expect("upsert accounts");
        client
            .storage()
            .upsert_tags(tags)
            .await
            .expect("upsert tags");
        client
            .storage()
            .upsert_instruments(instruments)
            .await
            .expect("upsert instruments");
        client
            .storage()
            .upsert_transactions(transactions)
            .await
            .expect("upsert transactions");
        client
            .storage()
            .upsert_merchants(merchants)
            .await
            .expect("upsert merchants");
        client
            .storage()
            .upsert_budgets(budgets)
            .await
            .expect("upsert budgets");
        client
            .storage()
            .upsert_reminders(reminders)
            .await
            .expect("upsert reminders");

        ZenMoneyMcpServer::new(client)
    }

    /// Extracts the text string from a successful `CallToolResult`.
    fn result_text(result: &CallToolResult) -> &str {
        assert!(
            !result.is_error.unwrap_or(false),
            "result should not be error"
        );
        result.content[0]
            .as_text()
            .expect("expected text content")
            .text
            .as_str()
    }

    #[tokio::test]
    async fn handler_list_accounts_all() {
        let server = build_test_server().await;
        let params = Parameters(ListAccountsParams { active_only: false });
        let result = server
            .list_accounts(params)
            .await
            .expect("should list accounts");
        let accounts: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse JSON");
        assert_eq!(accounts.len(), 2);
    }

    #[tokio::test]
    async fn handler_list_accounts_active_only() {
        let server = build_test_server().await;
        let params = Parameters(ListAccountsParams { active_only: true });
        let result = server.list_accounts(params).await.expect("should list");
        let accounts: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(accounts.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_transactions_default() {
        let server = build_test_server().await;
        let params = Parameters(ListTransactionsParams::default());
        let result = server
            .list_transactions(params)
            .await
            .expect("should list transactions");
        let txs: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(txs.len(), 3);
    }

    #[tokio::test]
    async fn handler_list_transactions_filter_expense() {
        let server = build_test_server().await;
        let params = Parameters(ListTransactionsParams {
            transaction_type: Some(TransactionType::Expense),
            ..Default::default()
        });
        let result = server.list_transactions(params).await.expect("should list");
        let txs: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(txs.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_transactions_with_limit() {
        let server = build_test_server().await;
        let params = Parameters(ListTransactionsParams {
            limit: Some(1),
            ..Default::default()
        });
        let result = server.list_transactions(params).await.expect("should list");
        let txs: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(txs.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_transactions_sort_asc() {
        let server = build_test_server().await;
        let params = Parameters(ListTransactionsParams {
            sort: Some(SortDirection::Asc),
            ..Default::default()
        });
        let result = server.list_transactions(params).await.expect("should list");
        assert!(!result.is_error.unwrap_or(false));
    }

    #[tokio::test]
    async fn handler_list_transactions_uncategorized() {
        let server = build_test_server().await;
        let params = Parameters(ListTransactionsParams {
            uncategorized: Some(true),
            ..Default::default()
        });
        let result = server.list_transactions(params).await.expect("should list");
        let txs: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        // All sample transactions have no tags.
        assert_eq!(txs.len(), 3);
    }

    #[tokio::test]
    async fn handler_list_tags() {
        let server = build_test_server().await;
        let result = server.list_tags().await.expect("should list tags");
        let tags: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(tags.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_merchants() {
        let server = build_test_server().await;
        let result = server
            .list_merchants()
            .await
            .expect("should list merchants");
        let merchants: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(merchants.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_budgets_all() {
        let server = build_test_server().await;
        let params = Parameters(ListBudgetsParams { month: None });
        let result = server
            .list_budgets(params)
            .await
            .expect("should list budgets");
        let budgets: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(budgets.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_budgets_filter_month() {
        let server = build_test_server().await;
        let params = Parameters(ListBudgetsParams {
            month: Some("2024-06".to_owned()),
        });
        let result = server.list_budgets(params).await.expect("should list");
        let budgets: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(budgets.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_budgets_filter_no_match() {
        let server = build_test_server().await;
        let params = Parameters(ListBudgetsParams {
            month: Some("2025-01".to_owned()),
        });
        let result = server.list_budgets(params).await.expect("should list");
        let budgets: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert!(budgets.is_empty());
    }

    #[tokio::test]
    async fn handler_list_reminders() {
        let server = build_test_server().await;
        let result = server
            .list_reminders()
            .await
            .expect("should list reminders");
        let reminders: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(reminders.len(), 1);
    }

    #[tokio::test]
    async fn handler_list_instruments() {
        let server = build_test_server().await;
        let result = server
            .list_instruments()
            .await
            .expect("should list instruments");
        let instruments: Vec<serde_json::Value> =
            serde_json::from_str(result_text(&result)).expect("should parse");
        assert_eq!(instruments.len(), 2);
    }

    #[tokio::test]
    async fn handler_find_account_found() {
        let server = build_test_server().await;
        let params = Parameters(FindAccountParams {
            title: "main account".to_owned(),
        });
        let result = server.find_account(params).await.expect("should find");
        assert!(result_text(&result).contains("Main Account"));
    }

    #[tokio::test]
    async fn handler_find_account_not_found() {
        let server = build_test_server().await;
        let params = Parameters(FindAccountParams {
            title: "nonexistent".to_owned(),
        });
        let result = server.find_account(params).await.expect("should respond");
        assert!(result_text(&result).contains("No account found"));
    }

    #[tokio::test]
    async fn handler_find_tag_found() {
        let server = build_test_server().await;
        let params = Parameters(FindTagParams {
            title: "groceries".to_owned(),
        });
        let result = server.find_tag(params).await.expect("should find");
        assert!(result_text(&result).contains("Groceries"));
    }

    #[tokio::test]
    async fn handler_find_tag_not_found() {
        let server = build_test_server().await;
        let params = Parameters(FindTagParams {
            title: "nonexistent".to_owned(),
        });
        let result = server.find_tag(params).await.expect("should respond");
        assert!(result_text(&result).contains("No tag found"));
    }

    #[tokio::test]
    async fn handler_create_tag_existing_is_idempotent() {
        let server = build_test_server().await;
        let params = Parameters(sample_create_tag_params("gRoCeRiEs"));
        let result = server
            .create_tag(params)
            .await
            .expect("should return existing");
        let payload: serde_json::Value =
            serde_json::from_str(result_text(&result)).expect("should parse");
        let id = payload
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("response should include id");
        assert_eq!(id, "tag-1");

        let tags = server.client.tags().await.expect("should load tags");
        assert_eq!(tags.len(), 1);
    }

    #[tokio::test]
    async fn handler_create_category_alias_existing_is_idempotent() {
        let server = build_test_server().await;
        let params = Parameters(sample_create_tag_params("GROCERIES"));
        let result = server
            .create_category(params)
            .await
            .expect("should return existing");
        let payload: serde_json::Value =
            serde_json::from_str(result_text(&result)).expect("should parse");
        let title = payload
            .get("title")
            .and_then(serde_json::Value::as_str)
            .expect("response should include title");
        assert_eq!(title, "Groceries");

        let tags = server.client.tags().await.expect("should load tags");
        assert_eq!(tags.len(), 1);
    }

    #[tokio::test]
    async fn handler_create_tag_blank_title_errors() {
        let server = build_test_server().await;
        let params = Parameters(sample_create_tag_params("   "));
        let result = server.create_tag(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handler_create_tag_missing_parent_errors() {
        let server = build_test_server().await;
        let mut create_params = sample_create_tag_params("New category");
        create_params.parent_tag_id = Some("missing-parent".to_owned());
        let params = Parameters(create_params);
        let result = server.create_tag(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handler_get_instrument_found() {
        let server = build_test_server().await;
        let params = Parameters(GetInstrumentParams { id: 1 });
        let result = server.get_instrument(params).await.expect("should get");
        assert!(result_text(&result).contains("Russian Ruble"));
    }

    #[tokio::test]
    async fn handler_get_instrument_not_found() {
        let server = build_test_server().await;
        let params = Parameters(GetInstrumentParams { id: 999 });
        let result = server.get_instrument(params).await.expect("should respond");
        assert!(result_text(&result).contains("No instrument found"));
    }

    #[tokio::test]
    async fn handler_get_info() {
        let server = build_test_server().await;
        let info = server.get_info();
        assert!(info.instructions.is_some());
    }

    #[tokio::test]
    async fn handler_prepare_bulk_too_many_operations() {
        let server = build_test_server().await;
        let operations: Vec<BulkOperation> = (0..21_u32)
            .map(|idx| {
                BulkOperation::Create(CreateTransactionParams {
                    transaction_type: TransactionType::Expense,
                    date: "2024-06-15".to_owned(),
                    account_id: "acc-1".to_owned(),
                    amount: f64::from(idx) + 1.0,
                    to_account_id: None,
                    to_amount: None,
                    instrument_id: None,
                    to_instrument_id: None,
                    tag_ids: None,
                    payee: None,
                    comment: None,
                })
            })
            .collect();
        let params = Parameters(BulkOperationsParams { operations });
        let result = server.prepare_bulk_operations(params).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn handler_prepare_bulk_valid() {
        let server = build_test_server().await;
        let operations = vec![BulkOperation::Create(sample_create_params(
            TransactionType::Expense,
        ))];
        let params = Parameters(BulkOperationsParams { operations });
        let result = server
            .prepare_bulk_operations(params)
            .await
            .expect("should prepare");
        let text = result_text(&result);
        assert!(text.contains("preparation_id"));
        assert!(text.contains("\"created\": 1"));
    }

    #[tokio::test]
    async fn handler_execute_bulk_not_found() {
        let server = build_test_server().await;
        let params = Parameters(ExecuteBulkParams {
            preparation_id: "nonexistent".to_owned(),
        });
        let result = server.execute_bulk_operations(params).await;
        assert!(result.is_err());
    }
}

#[tool_handler]
impl<S: Storage + 'static> ServerHandler for ZenMoneyMcpServer<S> {
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
