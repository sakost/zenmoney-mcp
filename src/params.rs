//! Parameter structs for MCP tool inputs.
//!
//! Each struct derives [`serde::Deserialize`] and [`schemars::JsonSchema`]
//! so that `rmcp` can auto-generate JSON schemas for tool parameters.

use schemars::JsonSchema;
use serde::Deserialize;

/// Parameters for the `list_accounts` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ListAccountsParams {
    /// If `true`, return only non-archived accounts.
    #[serde(default)]
    pub(crate) active_only: bool,
}

/// Parameters for the `list_transactions` tool.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(crate) struct ListTransactionsParams {
    /// Start date (inclusive), format `YYYY-MM-DD`.
    pub(crate) date_from: Option<String>,
    /// End date (inclusive), format `YYYY-MM-DD`.
    pub(crate) date_to: Option<String>,
    /// Filter by account ID.
    pub(crate) account_id: Option<String>,
    /// Filter by tag ID.
    pub(crate) tag_id: Option<String>,
    /// Filter by payee substring (case-insensitive).
    pub(crate) payee: Option<String>,
    /// Filter by merchant ID.
    pub(crate) merchant_id: Option<String>,
    /// Minimum amount (income or outcome >= this value).
    pub(crate) min_amount: Option<f64>,
    /// Maximum amount (income and outcome <= this value).
    pub(crate) max_amount: Option<f64>,
    /// Maximum number of transactions to return.
    pub(crate) limit: Option<usize>,
}

/// Parameters for the `list_budgets` tool.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(crate) struct ListBudgetsParams {
    /// Filter by month, format `YYYY-MM`.
    pub(crate) month: Option<String>,
}

/// Parameters for the `find_account` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct FindAccountParams {
    /// Account title to search for (case-insensitive).
    pub(crate) title: String,
}

/// Parameters for the `find_tag` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct FindTagParams {
    /// Tag title to search for (case-insensitive).
    pub(crate) title: String,
}

/// Parameters for the `suggest_category` tool.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
pub(crate) struct SuggestCategoryParams {
    /// Payee name for category suggestion.
    pub(crate) payee: Option<String>,
    /// Comment text for category suggestion.
    pub(crate) comment: Option<String>,
}

/// Parameters for the `get_instrument` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct GetInstrumentParams {
    /// Instrument (currency) ID.
    pub(crate) id: i32,
}

/// Parameters for the `create_transaction` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct CreateTransactionParams {
    /// Transaction date, format `YYYY-MM-DD`.
    pub(crate) date: String,
    /// Outcome (source) account ID.
    pub(crate) outcome_account: String,
    /// Outcome amount (>= 0).
    pub(crate) outcome: f64,
    /// Outcome currency instrument ID.
    pub(crate) outcome_instrument: i32,
    /// Income (destination) account ID.
    pub(crate) income_account: String,
    /// Income amount (>= 0).
    pub(crate) income: f64,
    /// Income currency instrument ID.
    pub(crate) income_instrument: i32,
    /// Category tag IDs.
    pub(crate) tag_ids: Option<Vec<String>>,
    /// Payee name.
    pub(crate) payee: Option<String>,
    /// User comment.
    pub(crate) comment: Option<String>,
}

/// Parameters for the `delete_transaction` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct DeleteTransactionParams {
    /// Transaction ID to delete.
    pub(crate) id: String,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    reason = "test code uses expect for readability"
)]
mod tests {
    use super::{
        CreateTransactionParams, DeleteTransactionParams, FindAccountParams, FindTagParams,
        GetInstrumentParams, ListAccountsParams, ListBudgetsParams, ListTransactionsParams,
        SuggestCategoryParams,
    };

    #[test]
    fn list_accounts_defaults_to_all() {
        let json = r#"{}"#;
        let params: ListAccountsParams =
            serde_json::from_str(json).expect("should deserialize empty object");
        assert!(!params.active_only);
    }

    #[test]
    fn list_accounts_active_only() {
        let json = r#"{"active_only": true}"#;
        let params: ListAccountsParams =
            serde_json::from_str(json).expect("should deserialize active_only");
        assert!(params.active_only);
    }

    #[test]
    fn list_transactions_minimal() {
        let json = r#"{}"#;
        let params: ListTransactionsParams =
            serde_json::from_str(json).expect("should deserialize empty");
        assert!(params.date_from.is_none());
        assert!(params.date_to.is_none());
        assert!(params.account_id.is_none());
        assert!(params.tag_id.is_none());
        assert!(params.payee.is_none());
        assert!(params.merchant_id.is_none());
        assert!(params.min_amount.is_none());
        assert!(params.max_amount.is_none());
        assert!(params.limit.is_none());
    }

    #[test]
    fn list_transactions_full() {
        let json = r#"{
            "date_from": "2024-01-01",
            "date_to": "2024-12-31",
            "account_id": "acc-001",
            "tag_id": "tag-001",
            "payee": "Coffee",
            "merchant_id": "m-001",
            "min_amount": 100.0,
            "max_amount": 5000.0,
            "limit": 50
        }"#;
        let params: ListTransactionsParams =
            serde_json::from_str(json).expect("should deserialize full params");
        assert_eq!(params.date_from.as_deref(), Some("2024-01-01"));
        assert_eq!(params.date_to.as_deref(), Some("2024-12-31"));
        assert_eq!(params.account_id.as_deref(), Some("acc-001"));
        assert_eq!(params.tag_id.as_deref(), Some("tag-001"));
        assert_eq!(params.payee.as_deref(), Some("Coffee"));
        assert_eq!(params.merchant_id.as_deref(), Some("m-001"));
        assert!((params.min_amount.unwrap_or_default() - 100.0).abs() < f64::EPSILON);
        assert!((params.max_amount.unwrap_or_default() - 5000.0).abs() < f64::EPSILON);
        assert_eq!(params.limit, Some(50));
    }

    #[test]
    fn list_budgets_empty() {
        let json = r#"{}"#;
        let params: ListBudgetsParams =
            serde_json::from_str(json).expect("should deserialize empty");
        assert!(params.month.is_none());
    }

    #[test]
    fn list_budgets_with_month() {
        let json = r#"{"month": "2024-06"}"#;
        let params: ListBudgetsParams =
            serde_json::from_str(json).expect("should deserialize with month");
        assert_eq!(params.month.as_deref(), Some("2024-06"));
    }

    #[test]
    fn find_account_params() {
        let json = r#"{"title": "Main Account"}"#;
        let params: FindAccountParams =
            serde_json::from_str(json).expect("should deserialize title");
        assert_eq!(params.title, "Main Account");
    }

    #[test]
    fn find_tag_params() {
        let json = r#"{"title": "Groceries"}"#;
        let params: FindTagParams = serde_json::from_str(json).expect("should deserialize title");
        assert_eq!(params.title, "Groceries");
    }

    #[test]
    fn suggest_category_empty() {
        let json = r#"{}"#;
        let params: SuggestCategoryParams =
            serde_json::from_str(json).expect("should deserialize empty");
        assert!(params.payee.is_none());
        assert!(params.comment.is_none());
    }

    #[test]
    fn suggest_category_with_payee() {
        let json = r#"{"payee": "McDonalds", "comment": "lunch"}"#;
        let params: SuggestCategoryParams = serde_json::from_str(json).expect("should deserialize");
        assert_eq!(params.payee.as_deref(), Some("McDonalds"));
        assert_eq!(params.comment.as_deref(), Some("lunch"));
    }

    #[test]
    fn get_instrument_params() {
        let json = r#"{"id": 42}"#;
        let params: GetInstrumentParams =
            serde_json::from_str(json).expect("should deserialize id");
        assert_eq!(params.id, 42);
    }

    #[test]
    fn create_transaction_params() {
        let json = r#"{
            "date": "2024-06-15",
            "outcome_account": "acc-001",
            "outcome": 500.0,
            "outcome_instrument": 1,
            "income_account": "acc-002",
            "income": 0.0,
            "income_instrument": 1,
            "tag_ids": ["tag-food"],
            "payee": "Coffee Shop",
            "comment": "Morning coffee"
        }"#;
        let params: CreateTransactionParams =
            serde_json::from_str(json).expect("should deserialize");
        assert_eq!(params.date, "2024-06-15");
        assert_eq!(params.outcome_account, "acc-001");
        assert!((params.outcome - 500.0).abs() < f64::EPSILON);
        assert_eq!(params.outcome_instrument, 1);
        assert_eq!(params.income_account, "acc-002");
        assert!((params.income).abs() < f64::EPSILON);
        assert_eq!(params.income_instrument, 1);
        assert_eq!(
            params.tag_ids.as_deref(),
            Some(["tag-food".to_owned()].as_slice())
        );
        assert_eq!(params.payee.as_deref(), Some("Coffee Shop"));
        assert_eq!(params.comment.as_deref(), Some("Morning coffee"));
    }

    #[test]
    fn create_transaction_minimal() {
        let json = r#"{
            "date": "2024-01-01",
            "outcome_account": "acc-001",
            "outcome": 100.0,
            "outcome_instrument": 1,
            "income_account": "acc-001",
            "income": 0.0,
            "income_instrument": 1
        }"#;
        let params: CreateTransactionParams =
            serde_json::from_str(json).expect("should deserialize minimal");
        assert!(params.tag_ids.is_none());
        assert!(params.payee.is_none());
        assert!(params.comment.is_none());
    }

    #[test]
    fn delete_transaction_params() {
        let json = r#"{"id": "tx-001"}"#;
        let params: DeleteTransactionParams =
            serde_json::from_str(json).expect("should deserialize");
        assert_eq!(params.id, "tx-001");
    }
}
