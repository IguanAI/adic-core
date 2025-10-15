#[test]
fn test_simple() {
    println!("MINIMAL TEST - Running simple test");
    assert_eq!(1 + 1, 2);
    println!("MINIMAL TEST - Simple test complete");
}

#[tokio::test]
async fn test_async_simple() {
    use tokio::sync::oneshot;
    println!("MINIMAL TEST - Running async test");
    let (tx, rx) = oneshot::channel::<u8>();
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        let _ = tx.send(42);
    });
    let val = tokio::time::timeout(std::time::Duration::from_secs(1), rx)
        .await
        .expect("oneshot should complete in time")
        .expect("sender should send a value");
    assert_eq!(val, 42);
    println!("MINIMAL TEST - Async test complete");
}
