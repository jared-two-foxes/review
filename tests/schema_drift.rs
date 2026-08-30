use std::fs;

#[test]
fn checked_in_schemas_match_generated_schemas() {
    for (filename, generated) in review_protocol::generate_schemas() {
        let path = format!("schemas/review/{}", filename);
        let checked_in =
            fs::read_to_string(&path).unwrap_or_else(|_| panic!("{} should be checked in", path));

        assert_eq!(
            generated, checked_in,
            "checked-in {} drifts from generated output - run 'cargo run -p review-protocol --bin generate-schemas' to regenerate",
            filename
        );
    }
}
