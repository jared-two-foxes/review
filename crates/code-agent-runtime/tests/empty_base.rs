use code_agent_runtime::diff::{changed_files, FileStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::target::ReviewTarget;
use std::path::Path;

fn create_test_repo(root: &Path) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.rs"), "fn main() {}\n").unwrap();
    std::fs::write(root.join("README.md"), "# test\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["init"])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .status();
    let _ = std::process::Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=t@t.com",
            "commit",
            "-m",
            "init",
        ])
        .current_dir(root)
        .status();
}

#[test]
fn parse_empty_returns_empty_tree_variant() {
    let root = std::env::temp_dir().join("empty-target-parse-test");
    let _ = std::fs::remove_dir_all(&root);
    create_test_repo(&root);
    let repo = GitRepo::open(&root).unwrap();
    let target = ReviewTarget::parse(&repo, ":empty").unwrap();
    assert!(matches!(target, ReviewTarget::Empty));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn empty_base_vs_working_dir_shows_all_files_as_added() {
    let root = std::env::temp_dir().join("empty-target-workdir-test");
    let _ = std::fs::remove_dir_all(&root);
    create_test_repo(&root);
    let repo = GitRepo::open(&root).unwrap();
    let base = ReviewTarget::parse(&repo, ":empty").unwrap();
    let head = ReviewTarget::parse(&repo, ":working").unwrap();
    let files = changed_files(&repo, &base, &head).unwrap();
    assert!(files
        .iter()
        .any(|f| f.path == "src/main.rs" && f.status == FileStatus::Added));
    assert!(files
        .iter()
        .any(|f| f.path == "README.md" && f.status == FileStatus::Added));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn empty_base_vs_commit_shows_all_files_as_added() {
    let root = std::env::temp_dir().join("empty-target-commit-test");
    let _ = std::fs::remove_dir_all(&root);
    create_test_repo(&root);
    let repo = GitRepo::open(&root).unwrap();
    let base = ReviewTarget::parse(&repo, ":empty").unwrap();
    let head = ReviewTarget::parse(&repo, "HEAD").unwrap();
    let files = changed_files(&repo, &base, &head).unwrap();
    assert!(files
        .iter()
        .any(|f| f.path == "src/main.rs" && f.status == FileStatus::Added));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn empty_base_vs_index_shows_staged_files_as_added() {
    let root = std::env::temp_dir().join("empty-target-index-test");
    let _ = std::fs::remove_dir_all(&root);
    create_test_repo(&root);
    std::fs::write(root.join("staged.txt"), "staged\n").unwrap();
    let _ = std::process::Command::new("git")
        .args(["add", "staged.txt"])
        .current_dir(&root)
        .status();

    let repo = GitRepo::open(&root).unwrap();
    let base = ReviewTarget::parse(&repo, ":empty").unwrap();
    let head = ReviewTarget::parse(&repo, ":staged").unwrap();
    let files = changed_files(&repo, &base, &head).unwrap();

    assert!(files
        .iter()
        .any(|f| f.path == "staged.txt" && f.status == FileStatus::Added));
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn empty_base_label_is_stable() {
    let root = std::env::temp_dir().join("empty-target-label-test");
    let _ = std::fs::remove_dir_all(&root);
    create_test_repo(&root);
    let repo = GitRepo::open(&root).unwrap();
    let target = ReviewTarget::parse(&repo, ":empty").unwrap();
    assert_eq!(target.label(&repo), "empty");
    std::fs::remove_dir_all(&root).unwrap();
}

#[test]
fn empty_base_clean_repo_has_nonempty_diff_unlike_head() {
    let root = std::env::temp_dir().join("empty-target-contrast-test");
    let _ = std::fs::remove_dir_all(&root);
    create_test_repo(&root);
    let repo = GitRepo::open(&root).unwrap();

    // :empty vs :working → all files appear as added
    let empty_base = ReviewTarget::parse(&repo, ":empty").unwrap();
    let working_head = ReviewTarget::parse(&repo, ":working").unwrap();
    let empty_files = changed_files(&repo, &empty_base, &working_head).unwrap();
    assert!(
        !empty_files.is_empty(),
        ":empty vs :working must produce a non-empty diff on a clean repo"
    );

    // HEAD vs :working → empty diff (clean repo, no uncommitted changes)
    let head_base = ReviewTarget::parse(&repo, "HEAD").unwrap();
    let head_files = changed_files(&repo, &head_base, &working_head).unwrap();
    assert!(
        head_files.is_empty(),
        "HEAD vs :working must produce an empty diff on a clean repo"
    );

    std::fs::remove_dir_all(&root).unwrap();
}
