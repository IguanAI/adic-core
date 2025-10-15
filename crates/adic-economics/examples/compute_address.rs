use adic_economics::AccountAddress;
use adic_types::PublicKey;

fn main() {
    // New wallet public key from keygen
    let pubkey_hex = "2d0b5f3b5355e2103955556c501572b8912a491a18c18e4280efe43df9ba78b5";
    let pubkey_bytes: [u8; 32] = hex::decode(pubkey_hex)
        .expect("Invalid hex")
        .try_into()
        .expect("Wrong length");

    let pubkey = PublicKey::from_bytes(pubkey_bytes);
    let address = AccountAddress::from_public_key(&pubkey);

    println!("=== New v4 Wallet Address ===");
    println!("Public Key (hex): {}", pubkey_hex);
    println!("Address (hex):    {}", hex::encode(address.as_bytes()));
    println!("Address (bech32): {}", address.to_bech32().expect("Failed to encode"));
}
