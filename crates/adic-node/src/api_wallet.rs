use crate::node::AdicNode;
use adic_economics::AdicAmount;
use axum::{
    http::StatusCode,
    response::{IntoResponse, Json, Response},
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

#[derive(Serialize)]
pub struct WalletInfo {
    pub address: String,
    pub public_key: String,
    pub node_id: String,
}

#[derive(Serialize)]
pub struct BalanceResponse {
    pub address: String,
    pub total: f64,
    pub available: f64,
    pub locked: f64,
}

#[derive(Deserialize)]
pub struct TransferRequest {
    pub from: String,      // Hex address
    pub to: String,        // Hex address
    pub amount: f64,       // Amount in ADIC
    pub signature: String, // Hex signature of the transfer details
}

#[derive(Serialize)]
pub struct TransferResponse {
    pub tx_hash: String,
    pub status: String,
}

#[derive(Deserialize)]
pub struct FaucetRequest {
    pub address: String, // Hex address
}

#[derive(Serialize)]
pub struct FaucetResponse {
    pub amount: f64,
    pub tx_hash: String,
}

#[derive(Deserialize)]
pub struct SignRequest {
    pub message: String, // Message to sign
}

#[derive(Serialize)]
pub struct SignResponse {
    pub signature: String, // Hex signature
}

#[derive(Serialize)]
pub struct ErrorResponse {
    pub error: String,
}

pub async fn get_wallet_info(node: Arc<AdicNode>) -> Response {
    let address = node
        .wallet_address()
        .to_bech32()
        .unwrap_or_else(|_| hex::encode(node.wallet_address().as_bytes()));

    let wallet_info = WalletInfo {
        address,
        public_key: hex::encode(node.public_key().as_bytes()),
        node_id: node.node_id(),
    };

    (StatusCode::OK, Json(wallet_info)).into_response()
}

pub async fn get_balance(address_hex: String, node: Arc<AdicNode>) -> Response {
    // Parse address - support both bech32 and hex formats
    let address = if address_hex.starts_with("adic") {
        match adic_economics::AccountAddress::from_string(&address_hex) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&address_hex) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    };

    // Get balances
    let balance_manager = &node.economics.balances;

    match balance_manager.get_balance(address).await {
        Ok(total) => {
            let locked = balance_manager
                .get_locked_balance(address)
                .await
                .unwrap_or(AdicAmount::ZERO);
            let available = total.saturating_sub(locked);

            let display_address = address.to_bech32().unwrap_or_else(|_| address_hex.clone());

            let response = BalanceResponse {
                address: display_address,
                total: total.to_adic(),
                available: available.to_adic(),
                locked: locked.to_adic(),
            };

            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: format!("Failed to get balance: {}", e),
            }),
        )
            .into_response(),
    }
}

pub async fn transfer(node: Arc<AdicNode>, req: TransferRequest) -> Response {
    // Parse addresses - support both bech32 and hex formats
    let from_address = if req.from.starts_with("adic") {
        match adic_economics::AccountAddress::from_string(&req.from) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid from address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&req.from) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid from address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    };

    let to_address = if req.to.starts_with("adic") {
        match adic_economics::AccountAddress::from_string(&req.to) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid to address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&req.to) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid to address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    };

    // Verify signature
    // Create the message that should have been signed
    let message = format!("{}:{}:{}", req.from, req.to, req.amount);

    // Decode the signature
    let signature_bytes = match hex::decode(&req.signature) {
        Ok(bytes) if bytes.len() == 64 => bytes,
        _ => {
            return (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Invalid signature format".to_string(),
                }),
            )
                .into_response();
        }
    };

    // Verify signature based on wallet type
    if from_address == node.wallet_address() {
        // For transfers from the node's own wallet, verify with the node's keypair
        let our_signature = node.wallet().sign(message.as_bytes());

        if our_signature.as_bytes() != signature_bytes.as_slice() {
            return (
                StatusCode::UNAUTHORIZED,
                Json(ErrorResponse {
                    error: "Invalid signature for transfer".to_string(),
                }),
            )
                .into_response();
        }
    } else {
        // For external wallets, look up the public key from registry
        match node.wallet_registry.get_public_key(&from_address).await {
            Some(public_key) => {
                // Verify the signature using ed25519-dalek directly
                use ed25519_dalek::{Signature as DalekSignature, Verifier, VerifyingKey};

                let verifying_key = match VerifyingKey::from_bytes(public_key.as_bytes()) {
                    Ok(key) => key,
                    Err(_) => {
                        return (
                            StatusCode::INTERNAL_SERVER_ERROR,
                            Json(ErrorResponse {
                                error: "Invalid public key for wallet".to_string(),
                            }),
                        )
                            .into_response();
                    }
                };

                let mut sig_array = [0u8; 64];
                sig_array.copy_from_slice(&signature_bytes);
                let dalek_sig = DalekSignature::from_bytes(&sig_array);

                if verifying_key
                    .verify(message.as_bytes(), &dalek_sig)
                    .is_err()
                {
                    return (
                        StatusCode::UNAUTHORIZED,
                        Json(ErrorResponse {
                            error: "Invalid signature for external wallet transfer".to_string(),
                        }),
                    )
                        .into_response();
                }

                // Mark wallet as used
                let _ = node.wallet_registry.mark_used(&from_address).await;
            }
            None => {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ErrorResponse {
                        error: format!(
                            "Wallet {} is not registered. Please register the wallet first.",
                            req.from
                        ),
                    }),
                )
                    .into_response();
            }
        }
    }

    let amount = AdicAmount::from_adic(req.amount);

    // Execute transfer
    match node
        .economics
        .balances
        .transfer(from_address, to_address, amount)
        .await
    {
        Ok(()) => {
            // Generate transaction hash
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            use std::hash::{Hash, Hasher};
            req.from.hash(&mut hasher);
            req.to.hash(&mut hasher);
            req.amount.to_bits().hash(&mut hasher);
            let tx_hash = format!("{:016x}", hasher.finish());

            let response = TransferResponse {
                tx_hash,
                status: "success".to_string(),
            };

            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Transfer failed: {}", e),
            }),
        )
            .into_response(),
    }
}

pub async fn request_faucet(node: Arc<AdicNode>, req: FaucetRequest) -> Response {
    // Parse address - support both bech32 and hex formats
    let address = if req.address.starts_with("adic") {
        match adic_economics::AccountAddress::from_string(&req.address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    } else {
        match crate::genesis::account_address_from_hex(&req.address) {
            Ok(addr) => addr,
            Err(e) => {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("Invalid address: {}", e),
                    }),
                )
                    .into_response();
            }
        }
    };

    // Get faucet address
    let faucet_address = crate::genesis::account_address_from_hex(
        &crate::genesis::derive_address_from_node_id("faucet"),
    )
    .unwrap();

    let amount = AdicAmount::from_adic(100.0); // 100 ADIC per request

    // Execute transfer from faucet
    match node
        .economics
        .balances
        .transfer(faucet_address, address, amount)
        .await
    {
        Ok(()) => {
            // Generate transaction hash
            let tx_hash = format!("{:016x}", rand::random::<u64>());

            let response = FaucetResponse {
                amount: amount.to_adic(),
                tx_hash,
            };

            (StatusCode::OK, Json(response)).into_response()
        }
        Err(e) => (
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: format!("Faucet unavailable: {}", e),
            }),
        )
            .into_response(),
    }
}

pub async fn sign_message(node: Arc<AdicNode>, req: SignRequest) -> Response {
    // Sign the message with the node wallet
    let signature = node.wallet().sign(req.message.as_bytes());

    let response = SignResponse {
        signature: hex::encode(signature.as_bytes()),
    };

    (StatusCode::OK, Json(response)).into_response()
}
