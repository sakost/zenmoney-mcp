//! Parameter structs for MCP tool inputs.
//!
//! Each struct derives [`serde::Deserialize`] and [`schemars::JsonSchema`]
//! so that `rmcp` can auto-generate JSON schemas for tool parameters.

use schemars::JsonSchema;
use serde::Deserialize;

/// Type of financial transaction.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum TransactionType {
    /// Money spent from an account.
    Expense,
    /// Money received into an account.
    Income,
    /// Money moved between two accounts.
    Transfer,
}

/// Sort direction for listing results.
#[derive(Debug, Clone, Default, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum SortDirection {
    /// Newest first.
    #[default]
    Desc,
    /// Oldest first.
    Asc,
}

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
    /// Maximum number of transactions to return (default 100, max 500).
    pub(crate) limit: Option<usize>,
    /// Number of transactions to skip (for pagination, default 0).
    #[serde(default)]
    pub(crate) offset: Option<usize>,
    /// If `true`, return only uncategorized transactions (no tags).
    pub(crate) uncategorized: Option<bool>,
    /// Filter by transaction type: expense, income, or transfer.
    pub(crate) transaction_type: Option<TransactionType>,
    /// Sort direction by date (default: desc = newest first).
    pub(crate) sort: Option<SortDirection>,
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
    /// Type of transaction: expense, income, or transfer.
    pub(crate) transaction_type: TransactionType,
    /// Transaction date, format `YYYY-MM-DD`.
    pub(crate) date: String,
    /// Primary account ID. For expense: source account. For income: destination account.
    /// For transfer: source account.
    pub(crate) account_id: String,
    /// Transaction amount (positive number).
    pub(crate) amount: f64,
    /// Destination account ID (required for transfers).
    pub(crate) to_account_id: Option<String>,
    /// Destination amount for transfers with currency conversion (defaults to `amount`).
    pub(crate) to_amount: Option<f64>,
    /// Override currency instrument ID for the primary account (auto-resolved from account if omitted).
    pub(crate) instrument_id: Option<i32>,
    /// Override currency instrument ID for the destination account (auto-resolved if omitted).
    pub(crate) to_instrument_id: Option<i32>,
    /// Category tag IDs.
    pub(crate) tag_ids: Option<Vec<String>>,
    /// Payee name.
    pub(crate) payee: Option<String>,
    /// User comment.
    pub(crate) comment: Option<String>,
}

/// Parameters for the `update_transaction` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct UpdateTransactionParams {
    /// Transaction ID to update.
    pub(crate) id: String,
    /// New date, format `YYYY-MM-DD`.
    pub(crate) date: Option<String>,
    /// New amount (applied to the appropriate side based on transaction type).
    pub(crate) amount: Option<f64>,
    /// New destination amount (for transfers with currency conversion).
    pub(crate) to_amount: Option<f64>,
    /// New primary account ID.
    pub(crate) account_id: Option<String>,
    /// New destination account ID (for transfers).
    pub(crate) to_account_id: Option<String>,
    /// New category tag IDs.
    pub(crate) tag_ids: Option<Vec<String>>,
    /// New payee name (empty string clears it).
    pub(crate) payee: Option<String>,
    /// New comment (empty string clears it).
    pub(crate) comment: Option<String>,
}

/// A single operation within a bulk request.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
#[serde(tag = "operation", rename_all = "snake_case")]
pub(crate) enum BulkOperation {
    /// Create a new transaction.
    Create(CreateTransactionParams),
    /// Update an existing transaction.
    Update(UpdateTransactionParams),
    /// Delete an existing transaction.
    Delete(DeleteTransactionParams),
}

/// Parameters for the `bulk_operations` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct BulkOperationsParams {
    /// List of operations to perform.
    pub(crate) operations: Vec<BulkOperation>,
}

/// Parameters for the `delete_transaction` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct DeleteTransactionParams {
    /// Transaction ID to delete.
    pub(crate) id: String,
}

/// Parameters for the `execute_bulk_operations` tool.
#[derive(Debug, Clone, Deserialize, JsonSchema)]
pub(crate) struct ExecuteBulkParams {
    /// Preparation ID returned by `prepare_bulk_operations`.
    pub(crate) preparation_id: String,
}

#[cfg(test)]
#[allow(
    clippy::expect_used,
    clippy::missing_docs_in_private_items,
    reason = "test code uses expect for readability"
)]
mod tests {
    use super::{
        BulkOperation, BulkOperationsParams, CreateTransactionParams, DeleteTransactionParams,
        ExecuteBulkParams, FindAccountParams, FindTagParams, GetInstrumentParams,
        ListAccountsParams, ListBudgetsParams, ListTransactionsParams, SuggestCategoryParams,
        UpdateTransactionParams,
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
        assert!(params.offset.is_none());
        assert!(params.uncategorized.is_none());
        assert!(params.transaction_type.is_none());
        assert!(params.sort.is_none());
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
            "limit": 50,
            "offset": 10
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
        assert_eq!(params.offset, Some(10));
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
    fn create_transaction_expense() {
        let json = r#"{
            "transaction_type": "expense",
            "date": "2024-06-15",
            "account_id": "acc-001",
            "amount": 500.0,
            "tag_ids": ["tag-food"],
            "payee": "Coffee Shop",
            "comment": "Morning coffee"
        }"#;
        let params: CreateTransactionParams =
            serde_json::from_str(json).expect("should deserialize");
        assert_eq!(params.date, "2024-06-15");
        assert_eq!(params.account_id, "acc-001");
        assert!((params.amount - 500.0).abs() < f64::EPSILON);
        assert!(params.to_account_id.is_none());
        assert_eq!(
            params.tag_ids.as_deref(),
            Some(["tag-food".to_owned()].as_slice())
        );
        assert_eq!(params.payee.as_deref(), Some("Coffee Shop"));
    }

    #[test]
    fn create_transaction_transfer() {
        let json = r#"{
            "transaction_type": "transfer",
            "date": "2024-01-01",
            "account_id": "acc-001",
            "amount": 1000.0,
            "to_account_id": "acc-002",
            "to_amount": 15.0
        }"#;
        let params: CreateTransactionParams =
            serde_json::from_str(json).expect("should deserialize transfer");
        assert_eq!(params.to_account_id.as_deref(), Some("acc-002"));
        assert!((params.to_amount.unwrap_or_default() - 15.0).abs() < f64::EPSILON);
    }

    #[test]
    fn create_transaction_minimal() {
        let json = r#"{
            "transaction_type": "income",
            "date": "2024-01-01",
            "account_id": "acc-001",
            "amount": 100.0
        }"#;
        let params: CreateTransactionParams =
            serde_json::from_str(json).expect("should deserialize minimal");
        assert!(params.tag_ids.is_none());
        assert!(params.payee.is_none());
        assert!(params.comment.is_none());
        assert!(params.instrument_id.is_none());
    }

    #[test]
    fn update_transaction_params() {
        let json = r#"{
            "id": "tx-001",
            "amount": 200.0,
            "tag_ids": ["tag-food"],
            "payee": ""
        }"#;
        let params: UpdateTransactionParams =
            serde_json::from_str(json).expect("should deserialize");
        assert_eq!(params.id, "tx-001");
        assert!((params.amount.unwrap_or_default() - 200.0).abs() < f64::EPSILON);
        assert_eq!(params.payee.as_deref(), Some(""));
        assert!(params.date.is_none());
        assert!(params.comment.is_none());
    }

    #[test]
    fn bulk_operations_params() {
        let json = r#"{
            "operations": [
                {
                    "operation": "create",
                    "transaction_type": "expense",
                    "date": "2024-01-01",
                    "account_id": "acc-001",
                    "amount": 100.0
                },
                {
                    "operation": "update",
                    "id": "tx-001",
                    "amount": 200.0
                },
                {
                    "operation": "delete",
                    "id": "tx-002"
                }
            ]
        }"#;
        let params: BulkOperationsParams =
            serde_json::from_str(json).expect("should deserialize bulk");
        assert_eq!(params.operations.len(), 3);
        assert!(matches!(params.operations[0], BulkOperation::Create(_)));
        assert!(matches!(params.operations[1], BulkOperation::Update(_)));
        assert!(matches!(params.operations[2], BulkOperation::Delete(_)));
    }

    #[test]
    fn delete_transaction_params() {
        let json = r#"{"id": "tx-001"}"#;
        let params: DeleteTransactionParams =
            serde_json::from_str(json).expect("should deserialize");
        assert_eq!(params.id, "tx-001");
    }

    #[test]
    fn execute_bulk_params() {
        let json = r#"{"preparation_id": "prep-abc-123"}"#;
        let params: ExecuteBulkParams =
            serde_json::from_str(json).expect("should deserialize preparation_id");
        assert_eq!(params.preparation_id, "prep-abc-123");
    }
}
