//! Enriched response structs for MCP tool outputs.
//!
//! These structs resolve entity IDs to human-readable names, making
//! tool outputs more useful for LLM assistants.

use std::collections::HashMap;

use serde::Serialize;
use zenmoney_rs::models::{
    Account, Budget, Instrument, Interval, Merchant, Reminder, Tag, Transaction,
};

use crate::server::account_type_label;

/// Formats an [`Interval`] variant as a human-readable string.
fn interval_label(interval: Interval) -> String {
    match interval {
        Interval::Day => "Day",
        Interval::Week => "Week",
        Interval::Month => "Month",
        Interval::Year => "Year",
    }
    .to_owned()
}

/// Lookup maps for resolving entity IDs to display names.
#[derive(Debug, Default)]
pub(crate) struct LookupMaps {
    /// Account ID → title.
    accounts: HashMap<String, String>,
    /// Tag ID → title.
    tags: HashMap<String, String>,
    /// Instrument ID → currency symbol.
    instruments: HashMap<i32, String>,
    /// Account ID → instrument ID (for auto-resolving currency from account).
    account_instruments: HashMap<String, i32>,
}

impl LookupMaps {
    /// Resolves an account ID to its title.
    fn account_name(&self, id: &str) -> String {
        self.accounts
            .get(id)
            .cloned()
            .unwrap_or_else(|| id.to_owned())
    }

    /// Resolves a tag ID to its title.
    fn tag_name(&self, id: &str) -> String {
        self.tags.get(id).cloned().unwrap_or_else(|| id.to_owned())
    }

    /// Resolves an instrument ID to its currency symbol.
    fn instrument_symbol(&self, id: i32) -> String {
        self.instruments
            .get(&id)
            .cloned()
            .unwrap_or_else(|| id.to_string())
    }

    /// Resolves an account ID to its instrument (currency) ID.
    pub(crate) fn account_instrument(&self, id: &str) -> Option<i32> {
        self.account_instruments.get(id).copied()
    }
}

/// Enriched account for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct AccountResponse {
    /// Account ID.
    id: String,
    /// Display name.
    title: String,
    /// Account type.
    account_type: String,
    /// Current balance.
    balance: Option<f64>,
    /// Currency symbol.
    currency: String,
    /// Whether the account is archived.
    archive: bool,
    /// Whether to include in total balance.
    in_balance: bool,
}

impl AccountResponse {
    /// Creates an enriched account response from a raw account.
    pub(crate) fn from_account(account: &Account, maps: &LookupMaps) -> Self {
        let currency: String = account
            .instrument
            .map(|id| maps.instrument_symbol(id.into_inner()))
            .unwrap_or_default();
        Self {
            id: account.id.to_string(),
            title: account.title.clone(),
            account_type: account_type_label(account.kind).to_owned(),
            balance: account.balance,
            currency,
            archive: account.archive,
            in_balance: account.in_balance,
        }
    }
}

/// Enriched transaction for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TransactionResponse {
    /// Transaction ID.
    id: String,
    /// Transaction date.
    date: String,
    /// Income amount.
    income: f64,
    /// Income account name.
    income_account: String,
    /// Income currency symbol.
    income_currency: String,
    /// Outcome amount.
    outcome: f64,
    /// Outcome account name.
    outcome_account: String,
    /// Outcome currency symbol.
    outcome_currency: String,
    /// Category tag names.
    tags: Vec<String>,
    /// Payee name.
    payee: Option<String>,
    /// User comment.
    comment: Option<String>,
}

impl TransactionResponse {
    /// Creates an enriched transaction response from a raw transaction.
    pub(crate) fn from_transaction(tx: &Transaction, maps: &LookupMaps) -> Self {
        let tags: Vec<String> = tx
            .tag
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|tag_id| maps.tag_name(tag_id.as_inner()))
            .collect();
        Self {
            id: tx.id.to_string(),
            date: tx.date.to_string(),
            income: tx.income,
            income_account: maps.account_name(tx.income_account.as_inner()),
            income_currency: maps.instrument_symbol(tx.income_instrument.into_inner()),
            outcome: tx.outcome,
            outcome_account: maps.account_name(tx.outcome_account.as_inner()),
            outcome_currency: maps.instrument_symbol(tx.outcome_instrument.into_inner()),
            tags,
            payee: tx.payee.clone(),
            comment: tx.comment.clone(),
        }
    }
}

/// Enriched tag for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct TagResponse {
    /// Tag ID.
    id: String,
    /// Display name.
    title: String,
    /// Parent tag name (if nested).
    parent: Option<String>,
}

impl TagResponse {
    /// Creates an enriched tag response from a raw tag.
    pub(crate) fn from_tag(tag: &Tag, maps: &LookupMaps) -> Self {
        let parent: Option<String> = tag.parent.as_ref().map(|pid| maps.tag_name(pid.as_inner()));
        Self {
            id: tag.id.to_string(),
            title: tag.title.clone(),
            parent,
        }
    }
}

/// Enriched merchant for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct MerchantResponse {
    /// Merchant ID.
    id: String,
    /// Display name.
    title: String,
}

impl MerchantResponse {
    /// Creates a merchant response from a raw merchant.
    pub(crate) fn from_merchant(merchant: &Merchant) -> Self {
        Self {
            id: merchant.id.to_string(),
            title: merchant.title.clone(),
        }
    }
}

/// Enriched budget for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BudgetResponse {
    /// Budget month.
    date: String,
    /// Category tag name.
    tag: Option<String>,
    /// Income target.
    income: f64,
    /// Outcome target.
    outcome: f64,
}

impl BudgetResponse {
    /// Creates an enriched budget response from a raw budget.
    pub(crate) fn from_budget(budget: &Budget, maps: &LookupMaps) -> Self {
        let tag: Option<String> = budget.tag.as_ref().map(|tid| maps.tag_name(tid.as_inner()));
        Self {
            date: budget.date.to_string(),
            tag,
            income: budget.income,
            outcome: budget.outcome,
        }
    }
}

/// Enriched reminder for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct ReminderResponse {
    /// Reminder ID.
    id: String,
    /// Income amount.
    income: f64,
    /// Income account name.
    income_account: String,
    /// Outcome amount.
    outcome: f64,
    /// Outcome account name.
    outcome_account: String,
    /// Category tag names.
    tags: Vec<String>,
    /// Payee name.
    payee: Option<String>,
    /// Comment.
    comment: Option<String>,
    /// Start date.
    start_date: String,
    /// End date.
    end_date: Option<String>,
    /// Recurrence interval.
    interval: Option<String>,
}

impl ReminderResponse {
    /// Creates an enriched reminder response from a raw reminder.
    pub(crate) fn from_reminder(reminder: &Reminder, maps: &LookupMaps) -> Self {
        let tags: Vec<String> = reminder
            .tag
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|tid| maps.tag_name(tid.as_inner()))
            .collect();
        Self {
            id: reminder.id.to_string(),
            income: reminder.income,
            income_account: maps.account_name(reminder.income_account.as_inner()),
            outcome: reminder.outcome,
            outcome_account: maps.account_name(reminder.outcome_account.as_inner()),
            tags,
            payee: reminder.payee.clone(),
            comment: reminder.comment.clone(),
            start_date: reminder.start_date.to_string(),
            end_date: reminder.end_date.map(|d| d.to_string()),
            interval: reminder.interval.map(interval_label),
        }
    }
}

/// Enriched instrument for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct InstrumentResponse {
    /// Instrument ID.
    id: i32,
    /// Full name (e.g. "US Dollar").
    title: String,
    /// Short code (e.g. "USD").
    short_title: String,
    /// Currency symbol (e.g. "$").
    symbol: String,
    /// Exchange rate.
    rate: f64,
}

impl InstrumentResponse {
    /// Creates an instrument response from a raw instrument.
    pub(crate) fn from_instrument(instrument: &Instrument) -> Self {
        Self {
            id: instrument.id.into_inner(),
            title: instrument.title.clone(),
            short_title: instrument.short_title.clone(),
            symbol: instrument.symbol.clone(),
            rate: instrument.rate,
        }
    }
}

/// Response for a deleted transaction, showing what was removed.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct DeletedTransactionResponse {
    /// Status message.
    message: String,
    /// Details of the deleted transaction.
    transaction: TransactionResponse,
}

impl DeletedTransactionResponse {
    /// Creates a deleted transaction response.
    pub(crate) const fn new(message: String, transaction: TransactionResponse) -> Self {
        Self {
            message,
            transaction,
        }
    }
}

/// Response for bulk operations.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct BulkOperationsResponse {
    /// Number of transactions created.
    created: usize,
    /// Number of transactions updated.
    updated: usize,
    /// Number of transactions deleted.
    deleted: usize,
    /// Details of created and updated transactions.
    transactions: Vec<TransactionResponse>,
}

impl BulkOperationsResponse {
    /// Creates a bulk operations response.
    pub(crate) const fn new(
        created: usize,
        updated: usize,
        deleted: usize,
        transactions: Vec<TransactionResponse>,
    ) -> Self {
        Self {
            created,
            updated,
            deleted,
            transactions,
        }
    }
}

/// Response for `prepare_bulk_operations`, showing a preview of what will happen.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct PrepareResponse {
    /// Opaque ID to pass to `execute_bulk_operations`.
    pub(crate) preparation_id: String,
    /// Number of transactions to create.
    pub(crate) created: usize,
    /// Number of transactions to update.
    pub(crate) updated: usize,
    /// Number of transactions to delete.
    pub(crate) deleted: usize,
    /// Preview of transactions to create/update (enriched).
    pub(crate) transactions: Vec<TransactionResponse>,
    /// Preview of transactions to delete (enriched).
    pub(crate) deleted_transactions: Vec<TransactionResponse>,
}

/// Suggestion result for display.
#[derive(Debug, Clone, Serialize)]
pub(crate) struct SuggestResponse {
    /// Normalized payee name.
    payee: Option<String>,
    /// Suggested merchant ID.
    merchant: Option<String>,
    /// Suggested category tag names.
    tags: Vec<String>,
}

impl SuggestResponse {
    /// Creates a suggestion response with resolved tag names.
    pub(crate) fn from_suggest(
        resp: &zenmoney_rs::models::SuggestResponse,
        maps: &LookupMaps,
    ) -> Self {
        let tags: Vec<String> = resp
            .tag
            .as_deref()
            .unwrap_or_default()
            .iter()
            .map(|tid| maps.tag_name(tid.as_inner()))
            .collect();
        Self {
            payee: resp.payee.clone(),
            merchant: resp.merchant.as_ref().map(ToString::to_string),
            tags,
        }
    }
}

/// Builds lookup maps from the full set of entities.
pub(crate) fn build_lookup_maps(
    accounts: &[Account],
    tags: &[Tag],
    instruments: &[Instrument],
) -> LookupMaps {
    let mut maps = LookupMaps::default();
    for acc in accounts {
        let _existed = maps.accounts.insert(acc.id.to_string(), acc.title.clone());
        if let Some(instrument_id) = acc.instrument {
            let _existed_instrument = maps
                .account_instruments
                .insert(acc.id.to_string(), instrument_id.into_inner());
        }
    }
    for tag in tags {
        let _existed = maps.tags.insert(tag.id.to_string(), tag.title.clone());
    }
    for instr in instruments {
        let _existed = maps
            .instruments
            .insert(instr.id.into_inner(), instr.symbol.clone());
    }
    maps
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::shadow_reuse,
    clippy::missing_docs_in_private_items,
    reason = "test code uses expect and shadow reuse for readability"
)]
mod tests {
    use super::{AccountResponse, LookupMaps, TransactionResponse, build_lookup_maps};
    use chrono::{DateTime, NaiveDate};
    use zenmoney_rs::models::{
        Account, AccountId, AccountType, CompanyId, Instrument, InstrumentId, Tag, TagId,
        Transaction, TransactionId, UserId,
    };

    fn sample_maps() -> LookupMaps {
        let accounts = vec![Account {
            id: AccountId::new("acc-1".to_owned()),
            changed: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test"),
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
        }];
        let tags = vec![Tag {
            id: TagId::new("tag-1".to_owned()),
            changed: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test"),
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
        let instruments = vec![Instrument {
            id: InstrumentId::new(1),
            changed: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test"),
            title: "Russian Ruble".to_owned(),
            short_title: "RUB".to_owned(),
            symbol: "\u{20bd}".to_owned(),
            rate: 1.0,
        }];
        build_lookup_maps(&accounts, &tags, &instruments)
    }

    #[test]
    fn lookup_resolves_known_ids() {
        let maps = sample_maps();
        assert_eq!(maps.account_name("acc-1"), "Main Account");
        assert_eq!(maps.tag_name("tag-1"), "Groceries");
        assert_eq!(maps.instrument_symbol(1), "\u{20bd}");
    }

    #[test]
    fn lookup_falls_back_to_id() {
        let maps = sample_maps();
        assert_eq!(maps.account_name("unknown"), "unknown");
        assert_eq!(maps.tag_name("unknown"), "unknown");
        assert_eq!(maps.instrument_symbol(999), "999");
    }

    #[test]
    fn account_response_formats_correctly() {
        let maps = sample_maps();
        let account = Account {
            id: AccountId::new("acc-1".to_owned()),
            changed: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test"),
            user: UserId::new(1),
            role: None,
            instrument: Some(InstrumentId::new(1)),
            company: Some(CompanyId::new(4)),
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
        };
        let resp = AccountResponse::from_account(&account, &maps);
        assert_eq!(resp.title, "Main Account");
        assert_eq!(resp.currency, "\u{20bd}");
        assert!(!resp.archive);
    }

    #[test]
    fn transaction_response_resolves_names() {
        let maps = sample_maps();
        let tx = Transaction {
            id: TransactionId::new("tx-1".to_owned()),
            changed: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test"),
            created: DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp for test"),
            user: UserId::new(1),
            deleted: false,
            hold: None,
            income_instrument: InstrumentId::new(1),
            income_account: AccountId::new("acc-1".to_owned()),
            income: 0.0,
            outcome_instrument: InstrumentId::new(1),
            outcome_account: AccountId::new("acc-1".to_owned()),
            outcome: 500.0,
            tag: Some(vec![TagId::new("tag-1".to_owned())]),
            merchant: None,
            payee: Some("Test Payee".to_owned()),
            original_payee: None,
            comment: Some("test comment".to_owned()),
            date: NaiveDate::from_ymd_opt(2024, 6, 15).expect("valid date for test"),
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
        let resp = TransactionResponse::from_transaction(&tx, &maps);
        assert_eq!(resp.income_account, "Main Account");
        assert_eq!(resp.outcome_account, "Main Account");
        assert_eq!(resp.income_currency, "\u{20bd}");
        assert_eq!(resp.tags, vec!["Groceries"]);
        assert_eq!(resp.payee.as_deref(), Some("Test Payee"));
    }
}
