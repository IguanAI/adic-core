//! Integration tests for event streaming system
//!
//! Tests the full event flow from emission to delivery via WebSocket/SSE

use adic_node::events::{EventBus, EventPriority, NodeEvent};
use chrono::Utc;
use tokio::time::{timeout, Duration};

#[tokio::test]
async fn test_event_bus_broadcast() {
    // Create event bus
    let bus = EventBus::new();

    // Subscribe to events
    let (mut high_rx, mut medium_rx, mut low_rx) = bus.subscribe_all();

    // Emit high priority event
    bus.emit(NodeEvent::TipsUpdated {
        tips: vec!["abc123".to_string()],
        count: 1,
        timestamp: Utc::now(),
    });

    // Verify high priority channel receives it
    let result = timeout(Duration::from_millis(100), high_rx.recv()).await;
    assert!(result.is_ok());
    assert!(result.unwrap().is_ok());

    // Verify other channels don't receive it
    assert!(medium_rx.try_recv().is_err());
    assert!(low_rx.try_recv().is_err());
}

#[tokio::test]
async fn test_event_priority_routing() {
    let bus = EventBus::new();
    let (mut high_rx, mut medium_rx, mut low_rx) = bus.subscribe_all();

    // Test high priority event
    bus.emit(NodeEvent::MessageFinalized {
        message_id: "test123".to_string(),
        finality_type: adic_node::events::FinalityType::KCore,
        timestamp: Utc::now(),
    });
    assert!(high_rx.try_recv().is_ok());

    // Test medium priority event
    bus.emit(NodeEvent::DiversityUpdated {
        diversity_score: 0.75,
        axes: vec![],
        total_tips: 10,
        timestamp: Utc::now(),
    });
    assert!(medium_rx.try_recv().is_ok());

    // Test low priority event
    bus.emit(NodeEvent::AdmissibilityUpdated {
        c1_rate: 0.99,
        c2_rate: 0.98,
        c3_rate: 0.97,
        overall_rate: 0.98,
        sample_size: 100,
        timestamp: Utc::now(),
    });
    assert!(low_rx.try_recv().is_ok());
}

#[tokio::test]
async fn test_multiple_subscribers() {
    let bus = EventBus::new();

    // Create multiple subscribers
    let (mut high_rx1, _, _) = bus.subscribe_all();
    let (mut high_rx2, _, _) = bus.subscribe_all();
    let (mut high_rx3, _, _) = bus.subscribe_all();

    // Emit event
    bus.emit(NodeEvent::TipsUpdated {
        tips: vec!["test".to_string()],
        count: 1,
        timestamp: Utc::now(),
    });

    // All subscribers should receive the event
    assert!(high_rx1.try_recv().is_ok());
    assert!(high_rx2.try_recv().is_ok());
    assert!(high_rx3.try_recv().is_ok());
}

#[tokio::test]
async fn test_event_metrics_tracking() {
    let bus = EventBus::new();
    let (_rx1, _rx2, _rx3) = bus.subscribe_all();

    assert_eq!(bus.total_events_emitted(), 0);

    // Emit multiple events
    for i in 0..10 {
        bus.emit(NodeEvent::TipsUpdated {
            tips: vec![format!("tip_{}", i)],
            count: 1,
            timestamp: Utc::now(),
        });
    }

    assert_eq!(bus.total_events_emitted(), 10);
}

#[tokio::test]
async fn test_event_serialization() {
    // Test that all event types can be serialized to JSON
    let events = vec![
        NodeEvent::TipsUpdated {
            tips: vec!["abc".to_string()],
            count: 1,
            timestamp: Utc::now(),
        },
        NodeEvent::MessageFinalized {
            message_id: "test".to_string(),
            finality_type: adic_node::events::FinalityType::KCore,
            timestamp: Utc::now(),
        },
        NodeEvent::DiversityUpdated {
            diversity_score: 0.5,
            axes: vec![],
            total_tips: 5,
            timestamp: Utc::now(),
        },
        NodeEvent::EnergyUpdated {
            total_conflicts: 10,
            resolved_conflicts: 8,
            active_conflicts: 2,
            total_energy: 1.5,
            timestamp: Utc::now(),
        },
    ];

    for event in events {
        let json = serde_json::to_string(&event);
        assert!(json.is_ok(), "Failed to serialize event: {:?}", event);

        // Verify it can be deserialized back
        let deserialized = serde_json::from_str::<NodeEvent>(&json.unwrap());
        assert!(deserialized.is_ok(), "Failed to deserialize event");
    }
}

#[tokio::test]
async fn test_subscriber_count() {
    let bus = EventBus::new();

    assert_eq!(bus.subscriber_count(), 0);

    let (_rx1, _rx2, _rx3) = bus.subscribe_all();
    assert_eq!(bus.subscriber_count(), 3);

    let (_rx4, _rx5, _rx6) = bus.subscribe_all();
    assert_eq!(bus.subscriber_count(), 6);
}

#[tokio::test]
async fn test_event_priority_classification() {
    // Verify each event type is assigned the correct priority
    let high_event = NodeEvent::TipsUpdated {
        tips: vec![],
        count: 0,
        timestamp: Utc::now(),
    };
    assert_eq!(high_event.priority(), EventPriority::High);

    let medium_event = NodeEvent::DiversityUpdated {
        diversity_score: 0.0,
        axes: vec![],
        total_tips: 0,
        timestamp: Utc::now(),
    };
    assert_eq!(medium_event.priority(), EventPriority::Medium);

    let low_event = NodeEvent::AdmissibilityUpdated {
        c1_rate: 0.0,
        c2_rate: 0.0,
        c3_rate: 0.0,
        overall_rate: 0.0,
        sample_size: 0,
        timestamp: Utc::now(),
    };
    assert_eq!(low_event.priority(), EventPriority::Low);
}

#[tokio::test]
async fn test_event_type_names() {
    // Verify event type strings match expected dotted notation
    let event = NodeEvent::TipsUpdated {
        tips: vec![],
        count: 0,
        timestamp: Utc::now(),
    };
    assert_eq!(event.event_type(), "tips.update");

    let event = NodeEvent::MessageFinalized {
        message_id: "test".to_string(),
        finality_type: adic_node::events::FinalityType::KCore,
        timestamp: Utc::now(),
    };
    assert_eq!(event.event_type(), "finality.update");
}
