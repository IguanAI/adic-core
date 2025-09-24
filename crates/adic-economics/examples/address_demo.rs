use adic_economics::{address_encoding, AccountAddress};
use adic_types::PublicKey;

fn main() {
    println!("ADIC Address Format Demo");
    println!("========================\n");

    // Example 1: Create address from public key bytes
    let pubkey_bytes = [0x42; 32];
    let pubkey = PublicKey::from_bytes(pubkey_bytes);
    let address = AccountAddress::from_public_key(&pubkey);

    println!("Example 1: From Public Key");
    println!("  Public key (hex): {}", hex::encode(&pubkey_bytes));
    println!("  Address (bech32): {}", address.to_bech32().unwrap());
    println!("  Address (hex):    0x{}", hex::encode(address.as_bytes()));
    println!();

    // Example 2: Different addresses produce different encodings
    println!("Example 2: Different Addresses");
    for i in 0..3 {
        let mut bytes = [i * 0x11; 32];
        bytes[0] = i;
        let addr = AccountAddress::from_bytes(bytes);
        println!("  Address {}: {}", i + 1, addr.to_bech32().unwrap());
    }
    println!();

    // Example 3: Parse addresses from strings (both formats)
    println!("Example 3: Parsing Addresses");
    let bech32_str = address.to_bech32().unwrap();
    let hex_str = format!("0x{}", hex::encode(address.as_bytes()));

    println!("  Parsing bech32: {}", bech32_str);
    let parsed1 = AccountAddress::from_string(&bech32_str).unwrap();
    println!("    ✓ Successfully parsed");

    println!("  Parsing hex: {}", hex_str);
    let parsed2 = AccountAddress::from_string(&hex_str).unwrap();
    println!("    ✓ Successfully parsed");

    assert_eq!(parsed1.as_bytes(), parsed2.as_bytes());
    println!("    ✓ Both formats decode to same address");
    println!();

    // Example 4: Address validation
    println!("Example 4: Address Validation");
    let valid_addr = address.to_bech32().unwrap();
    let invalid_addrs = vec![
        "btc1invalidaddress", // Wrong prefix
        "adic1invalid",       // Too short
        "not_an_address",     // Invalid format
        "0x123",              // Invalid hex length
    ];

    println!("  Valid address: {}", valid_addr);
    println!("    ✓ {}", address_encoding::validate_address(&valid_addr));

    for invalid in invalid_addrs {
        println!("  Invalid: {}", invalid);
        println!("    ✗ {}", address_encoding::validate_address(invalid));
    }
    println!();

    // Example 5: Special addresses
    println!("Example 5: Special System Addresses");
    let treasury = AccountAddress::treasury();
    let liquidity = AccountAddress::liquidity_pool();
    let community = AccountAddress::community_grants();
    let genesis = AccountAddress::genesis_pool();

    println!("  Treasury:         {}", treasury.to_bech32().unwrap());
    println!("  Liquidity Pool:   {}", liquidity.to_bech32().unwrap());
    println!("  Community Grants: {}", community.to_bech32().unwrap());
    println!("  Genesis Pool:     {}", genesis.to_bech32().unwrap());
}
