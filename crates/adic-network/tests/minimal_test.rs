#[test]
fn test_simple() {
    println!("MINIMAL TEST - Running simple test");
    assert_eq!(1 + 1, 2);
    println!("MINIMAL TEST - Simple test complete");
}

#[tokio::test]
async fn test_async_simple() {
    println!("MINIMAL TEST - Running async test");
    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    println!("MINIMAL TEST - Async test complete");
}
