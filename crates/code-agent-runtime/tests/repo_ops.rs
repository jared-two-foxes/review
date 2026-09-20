use code_agent_runtime::diff::{changed_files, read_diff, FileStatus};
use code_agent_runtime::error::RepoError;
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::snapshot::{resolve_targets, snapshot_id};
use code_agent_runtime::target::ReviewTarget;
use git2::Oid;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::process::Command;
use tempfile::TempDir;

fn make_test_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();

    Command::new("git")
        .arg("init")
        .current_dir(dir.path())
        .output()
        .unwrap();
    // Create src dir FIRST, then write the file
    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    // Git needs user config to commit in a temp dir
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@test.com",
            "commit",
            "-m",
            "initial",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    dir
}

#[test]
fn open_resolves_root() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    assert!(repo.root().exists());
}

#[test]
fn canonicalize_valid_path() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let path = repo.canonicalize_path("src/main.rs").unwrap();
    assert!(path.ends_with("src/main.rs"));
}

#[test]
fn canonicalize_rejects_escape() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let result = repo.canonicalize_path("../escape");
    assert!(result.is_err());
}

#[test]
fn repository_paths_are_rooted_and_canonicalized() {
    let dir = make_test_repo();
    let repo = GitRepo::open(&dir.path().join("src")).unwrap();
    let expected_root = dir.path().canonicalize().unwrap();

    assert_eq!(repo.root().canonicalize().unwrap(), expected_root);
    assert_eq!(
        repo.canonicalize_path("src/../src/main.rs").unwrap(),
        expected_root.join("src/main.rs")
    );

    assert!(matches!(
        repo.canonicalize_path("../escape"),
        Err(RepoError::PathEscape { .. })
    ));
}

#[test]
fn snapshot_id_is_deterministic() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let id1 = snapshot_id(&repo, &base, &head);
    let id2 = snapshot_id(&repo, &base, &head);
    assert_eq!(id1, id2);
    assert!(id1.starts_with("sha256:"));
}

#[test]
fn snapshot_id_encodes_the_resolved_commit_pair() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let base = Oid::from_str("0123456789abcdef0123456789abcdef01234567").unwrap();
    let head = Oid::from_str("fedcba9876543210fedcba9876543210fedcba98").unwrap();

    let first = snapshot_id(
        &repo,
        &ReviewTarget::Commit(base),
        &ReviewTarget::Commit(head),
    );
    let second = snapshot_id(
        &repo,
        &ReviewTarget::Commit(base),
        &ReviewTarget::Commit(head),
    );

    assert_eq!(
        first,
        "sha256:0123456789abcdef0123456789abcdef01234567:fedcba9876543210fedcba9876543210fedcba98"
    );
    assert_eq!(second, first);
}

fn make_test_repo_with_changes() -> TempDir {
    let dir = make_test_repo();
    // Create a second commit that modifies and adds files
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"hello\"); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/new.rs"), "pub fn new_fn() {}\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@test.com",
            "commit",
            "-m",
            "second",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    dir
}

#[test]
fn changed_files_enumerates_status() {
    let dir = make_test_repo_with_changes();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, "HEAD~1", "HEAD").unwrap();
    let changes = changed_files(&repo, &base, &head).unwrap();

    // src/new.rs was added
    assert!(changes
        .iter()
        .any(|c| c.path == "src/new.rs" && c.status == FileStatus::Added));
    // src/main.rs was modified
    assert!(changes
        .iter()
        .any(|c| c.path == "src/main.rs" && c.status == FileStatus::Modified));
}

fn make_test_repo_with_all_change_kinds() -> TempDir {
    let dir = make_test_repo();

    // Establish a base commit containing files that can be deleted and renamed.
    std::fs::write(dir.path().join("delete_me.txt"), "remove this\n").unwrap();
    std::fs::write(dir.path().join("rename_me.txt"), "move this\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@test.com",
            "commit",
            "-m",
            "base",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    // The head commit contains one instance of every supported status.
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"changed\"); }\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("added.txt"), "new file\n").unwrap();
    std::fs::remove_file(dir.path().join("delete_me.txt")).unwrap();
    std::fs::rename(
        dir.path().join("rename_me.txt"),
        dir.path().join("renamed.txt"),
    )
    .unwrap();
    Command::new("git")
        .args(["add", "-A"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@test.com",
            "commit",
            "-m",
            "head",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();
    dir
}

#[test]
fn changed_files_reports_added_modified_deleted_and_renamed_paths() {
    let dir = make_test_repo_with_all_change_kinds();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, "HEAD~1", "HEAD").unwrap();
    let changes = changed_files(&repo, &base, &head).unwrap();

    assert_eq!(changes.len(), 4);
    assert!(changes
        .iter()
        .any(|change| { change.path == "added.txt" && change.status == FileStatus::Added }));
    assert!(changes
        .iter()
        .any(|change| { change.path == "src/main.rs" && change.status == FileStatus::Modified }));
    assert!(changes
        .iter()
        .any(|change| { change.path == "delete_me.txt" && change.status == FileStatus::Deleted }));
    assert!(changes
        .iter()
        .any(|change| { change.path == "renamed.txt" && change.status == FileStatus::Renamed }));
}

#[test]
fn reverse_workdir_diff_reports_untracked_base_files_as_deleted() {
    let dir = make_test_repo();
    std::fs::write(dir.path().join("worktree_only.txt"), "worktree\n").unwrap();

    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, ":working", "HEAD").unwrap();
    let changes = changed_files(&repo, &base, &head).unwrap();

    assert!(changes
        .iter()
        .any(|change| change.path == "worktree_only.txt" && change.status == FileStatus::Deleted));
}

#[test]
fn read_diff_formats_hunks_and_marks_byte_limited_output() {
    let dir = make_test_repo();
    std::fs::write(
        dir.path().join("src/main.rs"),
        "fn main() { println!(\"changed 🦀🦀🦀\"); }\n",
    )
    .unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(dir.path())
        .output()
        .unwrap();
    Command::new("git")
        .args([
            "-c",
            "user.name=Test",
            "-c",
            "user.email=test@test.com",
            "commit",
            "-m",
            "diff",
        ])
        .current_dir(dir.path())
        .output()
        .unwrap();

    let repo = GitRepo::open(dir.path()).unwrap();
    let (base, head) = resolve_targets(&repo, "HEAD~1", "HEAD").unwrap();
    let complete = read_diff(&repo, &base, &head, "src/main.rs", usize::MAX).unwrap();
    assert!(
        complete
            .content
            .lines()
            .any(|line| line == "@@ -1 +1 @@" || line == "@@ -1,1 +1,1 @@"),
        "diff should contain a unified hunk header with line-number ranges: {}",
        complete.content
    );

    // Limit in the middle of the first crab so the result must keep only the
    // largest valid UTF-8 prefix that fits within the requested byte limit.
    let emoji_offset = complete.content.find('🦀').unwrap();
    let byte_limit = emoji_offset + 1;
    let limited = catch_unwind(AssertUnwindSafe(|| {
        read_diff(&repo, &base, &head, "src/main.rs", byte_limit)
    }));
    assert!(limited.is_ok(), "byte-limited diff must not panic on UTF-8");
    let limited = limited.unwrap().unwrap();
    assert!(limited.truncated);
    assert!(std::str::from_utf8(limited.content.as_bytes()).is_ok());

    let marker = format!("\n[truncated at {} bytes]", byte_limit);
    let retained = limited
        .content
        .strip_suffix(&marker)
        .expect("truncated diff should contain an explicit marker");
    assert_eq!(retained.as_bytes().len(), emoji_offset);
    assert_eq!(retained, &complete.content[..emoji_offset]);
    assert!(retained.as_bytes().len() < byte_limit);
}
