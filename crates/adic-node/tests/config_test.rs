use adic_node::NodeConfig;
use std::path::PathBuf;

#[test]
fn test_bootstrap_config_loads() {
    let config_path = PathBuf::from("../../bootstrap-config.toml");
    if !config_path.exists() {
        // Try alternative path
        let alt_path = PathBuf::from("bootstrap-config.toml");
        if !alt_path.exists() {
            eprintln!("Warning: bootstrap-config.toml not found, skipping test");
            return;
        }
    }

    let config = NodeConfig::from_file(&config_path).expect("Should load bootstrap-config.toml");

    // Verify bootstrap flag
    assert_eq!(
        config.node.bootstrap,
        Some(true),
        "Bootstrap node should have bootstrap=true"
    );

    // Verify genesis is present
    assert!(
        config.genesis.is_some(),
        "Bootstrap config should have genesis section"
    );

    let genesis = config.genesis.unwrap();

    // Verify genesis parameters
    assert_eq!(genesis.chain_id, "adic-dag-v1");
    assert_eq!(genesis.genesis_identities.len(), 4);
    assert!(!genesis.allocations.is_empty());

    // Verify total supply is 300.4M ADIC
    let total: u64 = genesis.allocations.iter().map(|(_, amt)| amt).sum();
    assert_eq!(
        total, 300_400_000,
        "Total genesis supply should be 300.4M ADIC"
    );

    // Verify genesis hash matches canonical
    let calculated_hash = genesis.calculate_hash();
    assert_eq!(
        calculated_hash, "e03dffb732c202021e35225771c033b1217b0e6241be360ad88f6d7ac43675f8",
        "Genesis hash should match canonical"
    );
}

#[test]
fn test_testnet_config_loads() {
    let config_path = PathBuf::from("../../testnet-config.toml");
    if !config_path.exists() {
        let alt_path = PathBuf::from("testnet-config.toml");
        if !alt_path.exists() {
            eprintln!("Warning: testnet-config.toml not found, skipping test");
            return;
        }
    }

    let config = NodeConfig::from_file(&config_path).expect("Should load testnet-config.toml");

    // Verify genesis is present
    assert!(
        config.genesis.is_some(),
        "Testnet config should have genesis section"
    );

    let genesis = config.genesis.unwrap();

    // Verify it's testnet genesis
    assert_eq!(genesis.chain_id, "adic-testnet");

    // Verify smaller test allocations
    let total: u64 = genesis.allocations.iter().map(|(_, amt)| amt).sum();
    assert!(
        total < 100_000,
        "Testnet should have smaller allocations than mainnet"
    );
}

#[test]
fn test_all_configs_parse() {
    let configs = vec![
        "../../bootstrap-config.toml",
        "../../testnet-config.toml",
        "../../adic-config.toml",
        "../../config/bootstrap-node.toml",
        "../../config/node1-config.toml",
        "../../config/node2-config.toml",
        "../../config/node3-config.toml",
    ];

    for config_path_str in configs {
        let config_path = PathBuf::from(config_path_str);
        if !config_path.exists() {
            eprintln!(
                "Warning: {} not found, trying without ../../",
                config_path_str
            );
            let short_path = config_path_str.trim_start_matches("../../");
            let alt_path = PathBuf::from(short_path);
            if !alt_path.exists() {
                eprintln!("Warning: {} not found either, skipping", short_path);
                continue;
            }
        }

        let result = NodeConfig::from_file(&config_path);
        assert!(
            result.is_ok(),
            "Config {} should parse successfully: {:?}",
            config_path_str,
            result.err()
        );

        let config = result.unwrap();

        // All configs should have genesis section now
        assert!(
            config.genesis.is_some(),
            "Config {} should have genesis section",
            config_path_str
        );

        let genesis = config.genesis.as_ref().unwrap();

        // Verify genesis is valid
        assert!(
            genesis.verify().is_ok(),
            "Genesis in {} should be valid",
            config_path_str
        );

        // Verify parameters match paper specs
        assert_eq!(genesis.parameters.p, 3);
        assert_eq!(genesis.parameters.d, 3);
        assert_eq!(genesis.parameters.rho, vec![2, 2, 1]);
        assert_eq!(genesis.parameters.q, 3);
        assert_eq!(genesis.parameters.k, 20);
        assert_eq!(genesis.parameters.depth_star, 12);
        assert_eq!(genesis.parameters.homology_window, 5);
    }
}
