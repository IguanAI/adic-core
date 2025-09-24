# Wallet API Documentation

## Overview
The ADIC Core wallet system supports both node-managed wallets and external wallet integration through a registry system. External wallets can register their public keys to enable signature verification for transactions.

## Wallet Registry Endpoints

### Register External Wallet
**POST** `/wallet/register`

Registers an external wallet's public key for a given address.

#### Request Body
```json
{
  "address": "adic1qz5y3...",  // Bech32 or hex format
  "public_key": "a3f2b8c9...", // Hex-encoded Ed25519 public key
  "signature": "9f8e7d6..."    // Hex-encoded signature of address bytes
}
```

#### Response
- **200 OK**: Wallet registered successfully
```json
{
  "status": "success",
  "message": "Wallet registered successfully"
}
```
- **400 Bad Request**: Invalid registration data or signature verification failed
```json
{
  "error": "Invalid signature"
}
```

### Get Wallet Public Key
**GET** `/wallet/public_key/:address`

Retrieves the registered public key for a wallet address.

#### Parameters
- `address`: Wallet address in bech32 (adic1...) or hex format

#### Response
- **200 OK**: Public key found
```json
{
  "address": "adic1qz5y3...",
  "public_key": "a3f2b8c9..."
}
```
- **404 Not Found**: Wallet not registered
```json
{
  "error": "Wallet not registered"
}
```

### List Registered Wallets
**GET** `/wallet/registered`

Lists all registered external wallets.

#### Response
- **200 OK**: List of registered wallets
```json
{
  "wallets": [
    {
      "address": "adic1qz5y3...",
      "public_key": "a3f2b8c9...",
      "registered_at": "2024-01-20T12:34:56Z",
      "last_used": "2024-01-20T13:45:00Z",
      "metadata": {
        "label": "My Hardware Wallet",
        "wallet_type": "hardware",
        "trusted": true
      }
    }
  ],
  "total": 1
}
```

### Check Wallet Registration
**GET** `/wallet/check/:address`

Check if a wallet address is registered.

#### Response
- **200 OK**: Registration status
```json
{
  "address": "adic1qz5y3...",
  "registered": true
}
```

### Get Wallet Info
**GET** `/wallet/info/:address`

Get detailed information about a registered wallet.

#### Response
- **200 OK**: Wallet information
```json
{
  "address": "adic1qz5y3...",
  "public_key": "a3f2b8c9...",
  "registered_at": "2024-01-20T12:34:56Z",
  "last_used": "2024-01-20T13:45:00Z",
  "metadata": {
    "label": "My Hardware Wallet",
    "wallet_type": "hardware",
    "trusted": true
  }
}
```
- **404 Not Found**: Wallet not in registry

### Unregister Wallet
**DELETE** `/wallet/unregister/:address`

Remove a wallet from the registry.

#### Response
- **200 OK**: Wallet unregistered
```json
{
  "status": "success",
  "message": "Wallet unregistered successfully"
}
```

### Registry Statistics
**GET** `/wallet/registry/stats`

Get statistics about the wallet registry.

#### Response
- **200 OK**: Registry statistics
```json
{
  "total_wallets": 10,
  "active_wallets": 5,
  "trusted_wallets": 3,
  "last_registration": "2024-01-20T12:34:56Z",
  "wallet_types": {
    "standard": 7,
    "hardware": 2,
    "multisig": 1
  }
}
```

### Export Node Wallet
**POST** `/wallet/export`

Export the node's wallet as encrypted JSON.

#### Request Body
```json
{
  "password": "encryption_password"
}
```

#### Response
- **200 OK**: Encrypted wallet JSON
```json
{
  "wallet": "{encrypted_json_string}",
  "format": "encrypted_json",
  "version": 3
}
```

### Import Wallet (Validation)
**POST** `/wallet/import`

Validate an imported wallet JSON (doesn't replace node wallet).

#### Request Body
```json
{
  "wallet_json": "{encrypted_json_string}",
  "password": "decryption_password"
}
```

#### Response
- **200 OK**: Wallet validated
```json
{
  "status": "success",
  "address": "adic1qz5y3...",
  "public_key": "a3f2b8c9...",
  "message": "Wallet successfully imported and validated"
}
```

## Transfer with External Wallets

### Transfer Endpoint (Updated)
**POST** `/wallet/transfer`

The transfer endpoint now supports both node wallets and registered external wallets.

#### Request Body (External Wallet)
```json
{
  "from": "adic1qz5y3...",      // Must be a registered external wallet
  "to": "adic1xyz9...",
  "amount": 100.0,
  "signature": "9f8e7d6...",     // Signature of transfer data
  "public_key": "a3f2b8c9..."    // Optional: public key if not registered
}
```

#### Signature Verification
For external wallets, the signature must be created by signing the following data:
```
from_address_bytes || to_address_bytes || amount_in_base_units || nonce
```

## CLI Wallet Management

### Export Wallet
```bash
adic wallet export --output exported_wallet.json --data-dir ./data --node-id node1
```
Exports the node's wallet to an encrypted JSON file. Prompts for password.

### Import Wallet
```bash
adic wallet import --input exported_wallet.json --data-dir ./data --node-id node1
```
Imports a wallet from an encrypted JSON file. Prompts for password.

### Show Wallet Info
```bash
adic wallet info --data-dir ./data --node-id node1
```
Displays wallet address, public key, and node ID.

## Wallet File Format

Exported wallets use the following JSON format:
```json
{
  "version": 3,
  "address": "adic1qz5y3...",
  "hex_address": "a1b2c3...",
  "public_key": "a3f2b8c9...",
  "encrypted_private_key": "base64_encrypted_data",
  "salt": "base64_salt",
  "created_at": "2024-01-20T12:34:56Z",
  "node_id": "node1"
}
```

### Encryption Details
- **Algorithm**: P-adic encryption (p=3, precision=32)
- **Key Derivation**: PBKDF2-like with SHA256, 100,000 iterations
- **Salt**: 32 bytes, randomly generated per export

## Security Considerations

1. **Password Protection**: All wallet exports are password-protected
2. **Signature Verification**: All external wallet operations require valid Ed25519 signatures
3. **Registration Required**: External wallets must register before performing transfers
4. **No Private Key Storage**: The registry only stores public keys, never private keys

## Example: Registering an External Wallet

```javascript
// Generate signature with your wallet's private key
const addressBytes = hexToBytes(addressHex);
const signature = ed25519.sign(addressBytes, privateKey);

// Register wallet
const response = await fetch('http://localhost:8080/wallet/register', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    address: 'adic1qz5y3...',
    public_key: publicKeyHex,
    signature: bytesToHex(signature)
  })
});
```

## Example: Transfer from External Wallet

```javascript
// Create transfer data
const transferData = Buffer.concat([
  fromAddressBytes,
  toAddressBytes,
  amountBytes,
  nonceBytes
]);

// Sign with private key
const signature = ed25519.sign(transferData, privateKey);

// Submit transfer
const response = await fetch('http://localhost:8080/wallet/transfer', {
  method: 'POST',
  headers: { 'Content-Type': 'application/json' },
  body: JSON.stringify({
    from: 'adic1qz5y3...',
    to: 'adic1xyz9...',
    amount: 100.0,
    signature: bytesToHex(signature)
  })
});
```