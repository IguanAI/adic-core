// Transaction history endpoint for wallet API

use crate::genesis;
use crate::node::AdicNode;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::sync::Arc;

#[derive(Serialize)]
pub struct Transaction {
    pub from: String,
    pub to: String,
    pub amount: f64,
    pub timestamp: DateTime<Utc>,
    pub tx_hash: String,
    pub status: String,
}

#[derive(Serialize)]
pub struct TransactionHistoryResponse {
    pub address: String,
    pub transactions: Vec<Transaction>,
    pub count: usize,
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn get_transaction_history(address_str: String, node: Arc<AdicNode>) -> Response {
    // Parse address - support both bech32 and hex formats
    let address = if address_str.starts_with("adic") {
        match adic_economics::AccountAddress::from_bech32(&address_str) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid bech32 address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    } else {
        match genesis::account_address_from_hex(&address_str) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid hex address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    };

    // Get transaction history from the balance manager
    let balance_manager = &node.economics.balances;
    let tx_records = match balance_manager.get_transaction_history(address).await {
        Ok(records) => records,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to get transaction history: {}", e),
                }),
            )
                .into_response();
        }
    };

    // Convert TransactionRecord to API Transaction format
    let transactions: Vec<Transaction> = tx_records
        .into_iter()
        .map(|record| Transaction {
            from: hex::encode(record.from.as_bytes()),
            to: hex::encode(record.to.as_bytes()),
            amount: record.amount.to_adic(),
            timestamp: record.timestamp,
            tx_hash: record.tx_hash,
            status: record.status,
        })
        .collect();

    let count = transactions.len();
    let response = TransactionHistoryResponse {
        address: address_str,
        transactions,
        count,
    };

    (StatusCode::OK, Json(response)).into_response()
}

#[derive(Serialize)]
pub struct AllTransactionsResponse {
    pub transactions: Vec<Transaction>,
    pub total: usize,
    pub limit: usize,
    pub offset: usize,
}

pub async fn get_all_transactions(
    limit: Option<usize>,
    offset: Option<usize>,
    node: Arc<AdicNode>,
) -> Response {
    let limit = limit.unwrap_or(100).min(1000); // Default 100, max 1000
    let offset = offset.unwrap_or(0);

    // Get all transactions from the balance manager
    let balance_manager = &node.economics.balances;
    let (tx_records, total) = match balance_manager
        .get_all_transactions_paginated(limit, offset)
        .await
    {
        Ok(result) => result,
        Err(e) => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: format!("Failed to get transactions: {}", e),
                }),
            )
                .into_response();
        }
    };

    // Convert TransactionRecord to API Transaction format
    let transactions: Vec<Transaction> = tx_records
        .into_iter()
        .map(|record| Transaction {
            from: hex::encode(record.from.as_bytes()),
            to: hex::encode(record.to.as_bytes()),
            amount: record.amount.to_adic(),
            timestamp: record.timestamp,
            tx_hash: record.tx_hash,
            status: record.status,
        })
        .collect();

    let response = AllTransactionsResponse {
        transactions,
        total,
        limit,
        offset,
    };

    (StatusCode::OK, Json(response)).into_response()
}
