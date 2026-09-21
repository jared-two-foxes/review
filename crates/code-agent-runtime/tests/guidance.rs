use code_agent_runtime::guidance::{GuidanceKind, collect_guidance_documents};

#[test]
fn collects_readme_and_root_agents() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("README.md"), "readme text").unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "root agent").unwrap();

    let docs = collect_guidance_documents(dir.path(), None);
    assert_eq!(docs.len(), 2);
    assert_eq!(docs[0].kind, GuidanceKind::Readme);
    assert!(docs[0].content.contains("readme text"));
    assert_eq!(docs[1].kind, GuidanceKind::Agents);
    assert!(docs[1].content.contains("root agent"));
}

#[test]
fn cascades_agents_from_root_to_target_directory() {
    let dir = tempfile::tempdir().unwrap();
    let nested = dir.path().join("src/nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "root").unwrap();
    std::fs::write(dir.path().join("src/AGENTS.md"), "src").unwrap();
    std::fs::write(dir.path().join("src/nested/AGENTS.md"), "nested").unwrap();

    let docs = collect_guidance_documents(dir.path(), Some("src/nested/file.rs"));
    let agent_contents: Vec<&str> = docs
        .iter()
        .filter(|doc| doc.kind == GuidanceKind::Agents)
        .map(|doc| doc.content.as_str())
        .collect();
    assert_eq!(agent_contents, vec!["root", "src", "nested"]);
}
