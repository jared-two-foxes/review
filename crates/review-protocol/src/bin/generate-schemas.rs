use std::fs;
use std::path::Path;

fn main() {
    let dir = Path::new("schemas/review");
    for (filename, schema) in review_protocol::generate_schemas() {
        let path = dir.join(filename);
        fs::create_dir_all(path.parent().unwrap()).expect("failed to create directories");
        fs::write(path, schema).expect("failed to write schema");
        println!("wrote schema: {}", filename);
    }
}
