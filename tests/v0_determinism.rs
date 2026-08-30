use std::fs;

use agent_protocol::{FixedClock, SequenceIdGenerator};
use review_app::run_review_with_sources;
use review_protocol::ReviewRequest;

#[test]
fn replay_is_byte_identical_to_golden() {
    let request: ReviewRequest = serde_json::from_str(
        &fs::read_to_string("tests/fixtures/v0/minimal-request.json")
            .expect("minimal request fixture should exist"),
    )
    .expect("minimal request fixture should parse");
    let golden = fs::read("tests/fixtures/v0/golden-result.json")
        .expect("golden result fixture should exist");

    let first = run_review_with_sources(
        &request,
        &FixedClock::new("2025-01-01T00:00:00Z"),
        &mut SequenceIdGenerator::new(["result-0001"]),
    );
    let second = run_review_with_sources(
        &request,
        &FixedClock::new("2025-01-01T00:00:00Z"),
        &mut SequenceIdGenerator::new(["result-0001"]),
    );

    assert_eq!(first, second);
    assert_eq!(first, golden);

    // Different injected sources must produce a different serialized result;
    // otherwise the replay seam is merely ignored by the application.
    let with_different_sources = run_review_with_sources(
        &request,
        &FixedClock::new("2030-06-15T12:34:56Z"),
        &mut SequenceIdGenerator::new(["result-9999"]),
    );
    assert_ne!(first, with_different_sources);
}
