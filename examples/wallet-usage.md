# Wallet Usage Examples

This guide provides practical examples of using the ADIC Core wallet functionality.

## Node Wallet Management

### Creating a Node Wallet
When you start an ADIC node for the first time, it automatically creates a wallet:

```bash
# Start node (creates wallet if not exists)
adic start --data-dir ./data

# View the created wallet
adic wallet info --data-dir ./data
```

### Setting a Custom Wallet Password
For production use, always set a strong wallet password:

```bash
export WALLET_PASSWORD="your-secure-password"
adic start --data-dir ./data
```

### Using a Custom Seed
To create deterministic wallets across nodes:

```bash
export WALLET_SEED="your-deterministic-seed-phrase"
adic start --data-dir ./data
```

## Wallet Import/Export

### Exporting a Wallet for Backup
```bash
# Export wallet to encrypted file
adic wallet export --output my-wallet-backup.json --data-dir ./data

# You'll be prompted for a password:
# Enter password to encrypt exported wallet: ********
```

### Importing a Wallet to a New Node
```bash
# Import wallet from backup
adic wallet import --input my-wallet-backup.json --data-dir ./new-node-data

# You'll be prompted for the password:
# Enter password to decrypt imported wallet: ********
```

### Migrating Between Nodes
```bash
# Export from node1
adic wallet export --output wallet.json --data-dir ./node1/data --node-id node1

# Import to node2
adic wallet import --input wallet.json --data-dir ./node2/data --node-id node2
```

## External Wallet Integration

### Python Example: Registering an External Wallet

```python
import ed25519
import requests
import binascii

# Generate or load your wallet keypair
signing_key = ed25519.SigningKey.generate()
verifying_key = signing_key.verifying_key

# Your wallet address (example)
address = "adic1qz5y3mku3shqdqhd5xun4j7dkqgqc5ljl5qgm6"
address_bytes = binascii.unhexlify(address[5:])  # Remove 'adic1' prefix

# Sign the address to prove ownership
signature = signing_key.sign(address_bytes)

# Register with the node
response = requests.post('http://localhost:8080/wallet/register', json={
    'address': address,
    'public_key': verifying_key.to_bytes().hex(),
    'signature': signature.hex()
})

print(f"Registration status: {response.json()}")
```

### JavaScript/TypeScript Example: External Wallet Transfer

```typescript
import { ed25519 } from '@noble/curves/ed25519';
import { hexToBytes, bytesToHex } from '@noble/hashes/utils';

class ExternalWallet {
  private privateKey: Uint8Array;
  private publicKey: Uint8Array;

  constructor(privateKeyHex: string) {
    this.privateKey = hexToBytes(privateKeyHex);
    this.publicKey = ed25519.getPublicKey(this.privateKey);
  }

  async register(nodeUrl: string, address: string): Promise<void> {
    // Sign address to prove ownership
    const addressBytes = hexToBytes(address.slice(5)); // Remove 'adic1'
    const signature = ed25519.sign(addressBytes, this.privateKey);

    const response = await fetch(`${nodeUrl}/wallet/register`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        address: address,
        public_key: bytesToHex(this.publicKey),
        signature: bytesToHex(signature)
      })
    });

    if (!response.ok) {
      throw new Error(`Registration failed: ${await response.text()}`);
    }
  }

  async transfer(
    nodeUrl: string,
    from: string,
    to: string,
    amount: number
  ): Promise<void> {
    // Create transfer signature
    const fromBytes = hexToBytes(from.slice(5));
    const toBytes = hexToBytes(to.slice(5));
    const amountBytes = new Uint8Array(8);
    new DataView(amountBytes.buffer).setBigUint64(0, BigInt(amount * 1e9), true);

    const nonce = Date.now();
    const nonceBytes = new Uint8Array(8);
    new DataView(nonceBytes.buffer).setBigUint64(0, BigInt(nonce), true);

    const dataToSign = new Uint8Array([
      ...fromBytes,
      ...toBytes,
      ...amountBytes,
      ...nonceBytes
    ]);

    const signature = ed25519.sign(dataToSign, this.privateKey);

    const response = await fetch(`${nodeUrl}/wallet/transfer`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({
        from: from,
        to: to,
        amount: amount,
        signature: bytesToHex(signature)
      })
    });

    if (!response.ok) {
      throw new Error(`Transfer failed: ${await response.text()}`);
    }
  }
}

// Usage
const wallet = new ExternalWallet('your_private_key_hex');
await wallet.register('http://localhost:8080', 'adic1qz5y3...');
await wallet.transfer(
  'http://localhost:8080',
  'adic1qz5y3...',
  'adic1xyz9...',
  100.0
);
```

### Rust Example: Using the Wallet Registry

```rust
use adic_node::wallet_registry::{WalletRegistry, WalletRegistrationRequest};
use adic_economics::AccountAddress;
use ed25519_dalek::{SigningKey, Signature, Signer};
use hex;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Create signing key
    let signing_key = SigningKey::generate(&mut rand::thread_rng());
    let verifying_key = signing_key.verifying_key();

    // Create address from public key
    let address = AccountAddress::from_public_key(&verifying_key.into());

    // Sign address bytes
    let signature = signing_key.sign(address.as_bytes());

    // Create registration request
    let registration = WalletRegistrationRequest {
        address: address.to_bech32()?,
        public_key: hex::encode(verifying_key.as_bytes()),
        signature: hex::encode(signature.to_bytes()),
    };

    // Register with node (assuming you have a registry instance)
    let registry = WalletRegistry::new(storage);
    registry.register_wallet(registration).await?;

    println!("Wallet registered successfully");
    Ok(())
}
```

## Security Best Practices

### 1. Password Management
```bash
# Never use default passwords in production
export WALLET_PASSWORD=$(openssl rand -base64 32)

# Store password securely (e.g., in a secrets manager)
echo $WALLET_PASSWORD | vault kv put secret/adic-wallet password=-
```

### 2. Backup Strategy
```bash
# Create encrypted backup with timestamp
adic wallet export \
  --output "wallet-backup-$(date +%Y%m%d-%H%M%S).json" \
  --data-dir ./data

# Store backup in multiple secure locations
# - Encrypted cloud storage
# - Hardware security module (HSM)
# - Offline cold storage
```

### 3. Key Rotation
```bash
# Periodically rotate wallet keys
# 1. Export old wallet
adic wallet export --output old-wallet.json --data-dir ./data

# 2. Generate new wallet
rm -rf ./data/wallet.json
export WALLET_SEED="new-secure-seed-$(date +%s)"
adic start --data-dir ./data

# 3. Transfer funds from old to new wallet
# (implement fund transfer logic)
```

## Troubleshooting

### Wallet Not Found
```bash
# Check wallet exists
ls -la ./data/wallet.json

# If missing, node will create one on start
adic start --data-dir ./data
```

### Wrong Password Error
```bash
# If you forget the password, you'll need to:
# 1. Use a backup with known password, or
# 2. Create a new wallet (funds in old wallet will be inaccessible)
```

### Registration Failed
```python
# Ensure signature is valid
def verify_registration(address, public_key, signature):
    verifying_key = ed25519.VerifyingKey(bytes.fromhex(public_key))
    address_bytes = binascii.unhexlify(address[5:])

    try:
        verifying_key.verify(bytes.fromhex(signature), address_bytes)
        return True
    except ed25519.BadSignatureError:
        return False
```

## Advanced Usage

### Multi-Signature Wallets
While not directly supported, you can implement multi-sig logic:

```typescript
// Register multiple wallets
const signers = [wallet1, wallet2, wallet3];
for (const signer of signers) {
  await signer.register(nodeUrl);
}

// Collect signatures for transfer
const signatures = [];
for (const signer of signers) {
  signatures.push(await signer.signTransfer(transferData));
}

// Submit with multiple signatures (requires custom endpoint)
await submitMultiSigTransfer(signatures, transferData);
```

### Hardware Wallet Integration
```python
# Example with Ledger device
from ledgerblue.comm import getDongle

dongle = getDongle(debug=True)

# Get public key from hardware wallet
apdu = bytes.fromhex("e002000000")  # Get public key command
response = dongle.exchange(apdu)
public_key = response[:32]

# Sign with hardware wallet
apdu = bytes.fromhex("e004000000") + data_to_sign
signature = dongle.exchange(apdu)

# Register hardware wallet with node
register_wallet(address, public_key.hex(), signature.hex())
```