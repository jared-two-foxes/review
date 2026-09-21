use code_agent_runtime::guidance::{GuidanceKind, collect_guidance_documents};
use std::process::Command;

fn init_repo(path: &std::path::Path) {
    let status = Command::new("git")
        .args(["init"])
        .current_dir(path)
        .status()
        .expect("git init");
    assert!(status.success());
}

#[test]
fn collects_readme_and_root_agents() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
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
    init_repo(dir.path());
    let nested = dir.path().join("src/nested");
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Directory summary:\n- src/ contains implementation code.",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/AGENTS.md"),
        "Directory summary:\n- nested/ contains module-specific guidance.",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/nested/AGENTS.md"), "nested").unwrap();

    let docs = collect_guidance_documents(dir.path(), Some("src/nested/file.rs"));
    let agent_contents: Vec<&str> = docs
        .iter()
        .filter(|doc| doc.kind == GuidanceKind::Agents)
        .map(|doc| doc.content.as_str())
        .collect();
    assert_eq!(agent_contents.len(), 3);
    assert!(agent_contents[0].contains("src/ contains implementation code"));
    assert!(agent_contents[1].contains("nested/ contains module-specific guidance"));
    assert_eq!(agent_contents[2], "nested");
}

#[test]
fn cascades_agents_for_nonexistent_target_file_path() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(dir.path().join("AGENTS.md"), "Focus areas include src/").unwrap();
    std::fs::write(
        dir.path().join("src/AGENTS.md"),
        "Focus areas include nested/",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/nested/AGENTS.md"), "nested").unwrap();

    let docs = collect_guidance_documents(dir.path(), Some("src/nested/new_file.rs"));
    let agent_contents: Vec<&str> = docs
        .iter()
        .filter(|doc| doc.kind == GuidanceKind::Agents)
        .map(|doc| doc.content.as_str())
        .collect();
    assert_eq!(
        agent_contents,
        vec![
            "Focus areas include src/",
            "Focus areas include nested/",
            "nested"
        ]
    );
}

#[test]
fn does_not_descend_when_parent_guidance_does_not_reference_child() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    std::fs::create_dir_all(dir.path().join("src/nested")).unwrap();
    std::fs::write(
        dir.path().join("AGENTS.md"),
        "Only docs/ needs review guidance.",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/AGENTS.md"), "src").unwrap();
    std::fs::write(dir.path().join("src/nested/AGENTS.md"), "nested").unwrap();

    let docs = collect_guidance_documents(dir.path(), Some("src/nested/new_file.rs"));
    let agent_contents: Vec<&str> = docs
        .iter()
        .filter(|doc| doc.kind == GuidanceKind::Agents)
        .map(|doc| doc.content.as_str())
        .collect();
    assert_eq!(agent_contents, vec!["Only docs/ needs review guidance."]);
}

#[test]
#[cfg(unix)]
fn ignores_guidance_symlinks_that_escape_the_repository() {
    let dir = tempfile::tempdir().unwrap();
    init_repo(dir.path());
    let external = tempfile::NamedTempFile::new().unwrap();
    std::fs::write(external.path(), "outside").unwrap();
    std::os::unix::fs::symlink(external.path(), dir.path().join("AGENTS.md")).unwrap();
    std::fs::write(dir.path().join("README.md"), "readme text").unwrap();

    let docs = collect_guidance_documents(dir.path(), None);
    assert_eq!(docs.len(), 1);
    assert_eq!(docs[0].kind, GuidanceKind::Readme);
    assert!(docs[0].content.contains("readme text"));
}
