use crate::address_encoding;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const ADIC_DECIMALS: u32 = 9;
pub const ADIC_BASE_UNIT: u64 = 1_000_000_000; // 10^9

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct AdicAmount(u64);

impl AdicAmount {
    pub const ZERO: Self = Self(0);
    pub const MAX_SUPPLY: Self = Self(1_000_000_000 * ADIC_BASE_UNIT); // 10^9 ADIC
    pub const GENESIS_SUPPLY: Self = Self(300_000_000 * ADIC_BASE_UNIT); // 3×10^8 ADIC

    pub fn from_adic(adic: f64) -> Self {
        Self((adic * ADIC_BASE_UNIT as f64) as u64)
    }

    pub fn from_base_units(units: u64) -> Self {
        Self(units)
    }

    pub fn to_adic(&self) -> f64 {
        self.0 as f64 / ADIC_BASE_UNIT as f64
    }

    pub fn to_base_units(&self) -> u64 {
        self.0
    }

    pub fn checked_add(&self, other: Self) -> Option<Self> {
        self.0.checked_add(other.0).map(Self)
    }

    pub fn checked_sub(&self, other: Self) -> Option<Self> {
        self.0.checked_sub(other.0).map(Self)
    }

    pub fn saturating_add(&self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0).min(Self::MAX_SUPPLY.0))
    }

    pub fn saturating_sub(&self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }
}

impl fmt::Display for AdicAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{:.9} ADIC", self.to_adic())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AccountAddress([u8; 32]);

impl AccountAddress {
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    pub fn from_public_key(pubkey: &adic_types::PublicKey) -> Self {
        Self(*pubkey.as_bytes())
    }

    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    pub fn to_bech32(&self) -> Result<String> {
        address_encoding::encode_address(&self.0)
    }

    pub fn from_bech32(address: &str) -> Result<Self> {
        let bytes = address_encoding::decode_address(address)?;
        Ok(Self(bytes))
    }

    pub fn from_string(address: &str) -> Result<Self> {
        // Try bech32 format first
        if address.starts_with("adic") {
            Self::from_bech32(address)
        } else if address_encoding::is_hex_address(address) {
            // Fall back to hex format for compatibility
            let bytes = address_encoding::from_hex_address(address)?;
            Ok(Self(bytes))
        } else {
            Err(anyhow::anyhow!("Invalid address format"))
        }
    }

    pub fn treasury() -> Self {
        Self([0xFF; 32])
    }

    pub fn liquidity_pool() -> Self {
        let mut bytes = [0xEE; 32];
        bytes[0] = 0x01;
        Self(bytes)
    }

    pub fn community_grants() -> Self {
        let mut bytes = [0xDD; 32];
        bytes[0] = 0x02;
        Self(bytes)
    }

    pub fn genesis_pool() -> Self {
        let mut bytes = [0xCC; 32];
        bytes[0] = 0x03;
        Self(bytes)
    }
}

impl fmt::Display for AccountAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Use bech32 format by default
        match self.to_bech32() {
            Ok(addr) => write!(f, "{}", addr),
            // Fall back to hex if encoding fails (shouldn't happen in practice)
            Err(_) => write!(f, "0x{}", hex::encode(&self.0[..8])),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllocationConfig {
    pub treasury_percent: f64,  // 20%
    pub liquidity_percent: f64, // 30%
    pub genesis_percent: f64,   // 50%
    pub treasury_multisig_threshold: u32,
    pub treasury_multisig_keys: Vec<AccountAddress>,
}

impl Default for AllocationConfig {
    fn default() -> Self {
        Self {
            treasury_percent: 0.20,
            liquidity_percent: 0.30,
            genesis_percent: 0.50,
            treasury_multisig_threshold: 2,
            treasury_multisig_keys: vec![],
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmissionSchedule {
    pub initial_rate: f64,    // 1% per year
    pub half_life_years: f64, // 6 years
    pub start_timestamp: i64,
    pub last_emission_timestamp: i64,
}

impl Default for EmissionSchedule {
    fn default() -> Self {
        Self {
            initial_rate: 0.01,
            half_life_years: 6.0,
            start_timestamp: chrono::Utc::now().timestamp(),
            last_emission_timestamp: chrono::Utc::now().timestamp(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransferEvent {
    pub from: AccountAddress,
    pub to: AccountAddress,
    pub amount: AdicAmount,
    pub timestamp: i64,
    pub reason: TransferReason,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum TransferReason {
    Genesis,
    Emission,
    Deposit,
    DepositRefund,
    DepositSlash,
    TreasuryTransfer,
    PoUWReward,
}
