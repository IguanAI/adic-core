use adic_types::{AdicParams, DEFAULT_P, DEFAULT_PRECISION};

#[test]
fn test_adic_params_default() {
    let params = AdicParams::default();

    // Check default values match expected production values
    assert_eq!(params.p, DEFAULT_P);
    assert_eq!(params.p, 3);
    assert_eq!(params.d, 3);
    assert_eq!(params.rho, vec![2, 2, 1]);
    assert_eq!(params.q, 3);
    assert_eq!(params.k, 20); // Production value from paper
    assert_eq!(params.depth_star, 12); // Production value from paper
    assert_eq!(params.delta, 5);
    assert_eq!(params.deposit, 0.1);
    assert_eq!(params.r_min, 1.0);
    assert_eq!(params.r_sum_min, 4.0);
    assert_eq!(params.lambda, 1.0);
    assert_eq!(params.beta, 0.5);
    assert_eq!(params.mu, 1.0);
    assert_eq!(params.gamma, 0.9);
}

#[test]
fn test_adic_params_serialization() {
    let params = AdicParams::default();

    // Serialize to JSON
    let json = serde_json::to_string(&params).unwrap();
    assert!(json.contains("\"p\":3"));
    assert!(json.contains("\"d\":3"));
    assert!(json.contains("\"deposit\":0.1"));

    // Deserialize from JSON
    let deserialized: AdicParams = serde_json::from_str(&json).unwrap();
    assert_eq!(params, deserialized);

    // Pretty print JSON
    let pretty_json = serde_json::to_string_pretty(&params).unwrap();
    assert!(pretty_json.contains("  \"p\": 3"));
}

#[test]
fn test_adic_params_custom() {
    let params = AdicParams {
        p: 5,
        d: 4,
        rho: vec![3, 2, 2, 1],
        q: 5,
        k: 10,
        depth_star: 8,
        delta: 10,
        deposit: 0.5,
        r_min: 2.0,
        r_sum_min: 8.0,
        lambda: 2.0,
        beta: 0.3,
        mu: 0.5,
        gamma: 0.95,
    };

    assert_eq!(params.p, 5);
    assert_eq!(params.d, 4);
    assert_eq!(params.rho.len(), 4);
    assert_eq!(params.gamma, 0.95);

    // Serialize and deserialize
    let json = serde_json::to_string(&params).unwrap();
    let deserialized: AdicParams = serde_json::from_str(&json).unwrap();
    assert_eq!(params, deserialized);
}

#[test]
fn test_adic_params_edge_cases() {
    // Minimum values
    let min_params = AdicParams {
        p: 2, // Minimum prime
        d: 1,
        rho: vec![1],
        q: 1,
        k: 1,
        depth_star: 0,
        delta: 0,
        deposit: 0.0,
        r_min: 0.0,
        r_sum_min: 0.0,
        lambda: 0.0,
        beta: 0.0,
        mu: 0.0,
        gamma: 0.0,
    };

    let json = serde_json::to_string(&min_params).unwrap();
    let deserialized: AdicParams = serde_json::from_str(&json).unwrap();
    assert_eq!(min_params, deserialized);

    // Large values
    let large_params = AdicParams {
        p: 997, // Large prime
        d: 100,
        rho: vec![10; 100],
        q: 1000,
        k: 10000,
        depth_star: 100000,
        delta: 1000000,
        deposit: 1000000.0,
        r_min: 999999.0,
        r_sum_min: 9999999.0,
        lambda: 100000.0,
        beta: 0.999999,
        mu: 100000.0,
        gamma: 0.999999,
    };

    let json = serde_json::to_string(&large_params).unwrap();
    let deserialized: AdicParams = serde_json::from_str(&json).unwrap();
    assert_eq!(large_params, deserialized);
}

#[test]
fn test_adic_params_gamma_validation() {
    // Gamma should be between 0 and 1 for reputation updates
    let params = AdicParams::default();
    assert!(params.gamma > 0.0);
    assert!(params.gamma < 1.0);

    // Test various gamma values
    let gamma_values = vec![0.0, 0.1, 0.5, 0.9, 0.99, 0.999];

    for gamma in gamma_values {
        let test_params = AdicParams {
            gamma,
            ..Default::default()
        };
        assert_eq!(test_params.gamma, gamma);

        // Serialize and check
        let json = serde_json::to_string(&test_params).unwrap();
        assert!(json.contains(&format!("\"gamma\":{}", gamma)));
    }
}

#[test]
fn test_adic_params_rho_vector() {
    // Test that rho vector length matches d
    let params = AdicParams::default();
    assert_eq!(params.rho.len(), params.d as usize);

    // Test custom rho vectors
    let custom_params = AdicParams {
        d: 5,
        rho: vec![4, 3, 2, 1, 0],
        ..AdicParams::default()
    };
    assert_eq!(custom_params.rho.len(), 5);
    assert_eq!(custom_params.rho[0], 4);
    assert_eq!(custom_params.rho[4], 0);

    // Empty rho vector (edge case)
    let empty_rho = AdicParams {
        d: 0,
        rho: vec![],
        ..AdicParams::default()
    };
    assert_eq!(empty_rho.rho.len(), 0);
}

#[test]
fn test_adic_params_equality() {
    let params1 = AdicParams::default();
    let params2 = AdicParams::default();
    let params3 = AdicParams {
        p: 5, // Different prime
        ..Default::default()
    };

    // Same parameters should be equal
    assert_eq!(params1, params2);

    // Different parameters should not be equal
    assert_ne!(params1, params3);

    // Clone should be equal
    let params1_clone = params1.clone();
    assert_eq!(params1, params1_clone);
}

#[test]
fn test_adic_params_constants() {
    // Test that constants are set correctly
    assert_eq!(DEFAULT_P, 3);
    assert_eq!(DEFAULT_PRECISION, 10);

    // Ensure defaults use these constants
    let params = AdicParams::default();
    assert_eq!(params.p, DEFAULT_P);
}

#[test]
fn test_adic_params_float_precision() {
    let params = AdicParams {
        deposit: 0.123456789,
        r_min: 1.111111111,
        r_sum_min: 4.444444444,
        lambda: 1.23456789,
        beta: 0.555555555,
        mu: 0.987654321,
        gamma: 0.9999999,
        ..AdicParams::default()
    };

    // Serialize and check float precision is preserved
    let json = serde_json::to_string(&params).unwrap();
    let deserialized: AdicParams = serde_json::from_str(&json).unwrap();

    // Floats should maintain reasonable precision
    assert!((params.deposit - deserialized.deposit).abs() < 1e-9);
    assert!((params.gamma - deserialized.gamma).abs() < 1e-9);
}

#[test]
fn test_adic_params_partial_deserialization() {
    // Test that we can deserialize with production values
    let minimal_json = r#"{
        "p": 3,
        "d": 3,
        "rho": [2, 2, 1],
        "q": 3,
        "k": 20,
        "depth_star": 12,
        "delta": 5,
        "deposit": 0.1,
        "r_min": 1.0,
        "r_sum_min": 4.0,
        "lambda": 1.0,
        "beta": 0.5,
        "mu": 1.0,
        "gamma": 0.9
    }"#;

    let params: AdicParams = serde_json::from_str(minimal_json).unwrap();
    assert_eq!(params, AdicParams::default());
}
