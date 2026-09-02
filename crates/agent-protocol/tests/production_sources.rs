use agent_protocol::{
    Clock, FixedClock, IdGenerator, RandomIdGenerator, SequenceIdGenerator, SystemClock,
};
use std::collections::HashSet;
use std::thread::sleep;
use std::time::Duration;

#[test]
fn fixed_clock_returns_its_configured_timestamp() {
    let clock = FixedClock::new("2025-01-01T00:00:00Z");
    assert_eq!(clock.now(), "2025-01-01T00:00:00Z");
}

#[test]
fn sequence_id_generator_yields_scripted_identifiers_in_order() {
    let mut generator = SequenceIdGenerator::new(["first", "second", "third"]);
    assert_eq!(generator.next_id(), "first");
    assert_eq!(generator.next_id(), "second");
    assert_eq!(generator.next_id(), "third");
}

#[test]
fn system_clock_returns_advancing_iso8601_utc_strings() {
    let clock = SystemClock::new();
    let first = clock.now();
    assert!(
        first.contains('T') && first.ends_with('Z'),
        "expected an ISO-8601 UTC timestamp, got {first}"
    );
    // Coarse system timers can return identical subsecond-stripped strings in
    // quick succession; retry briefly until the second reading differs.
    let mut second = first.clone();
    for _ in 0..100 {
        second = clock.now();
        if second != first {
            break;
        }
        sleep(Duration::from_millis(20));
    }
    assert_ne!(
        second, first,
        "successive SystemClock::now() calls must eventually return different values"
    );
}

#[test]
fn random_id_generator_produces_unique_nonempty_ids_across_many_calls() {
    let mut generator = RandomIdGenerator::new();
    let mut seen = HashSet::new();
    for _ in 0..10_000 {
        let id = generator.next_id();
        assert!(!id.is_empty(), "id must be non-empty");
        assert!(
            seen.insert(id),
            "duplicate id generated — uniqueness violated"
        );
    }
}
