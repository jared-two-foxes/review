use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::{SecurityError, SecurityPolicy, bound_matches};
use std::path::PathBuf;
use std::process::Command;
use tempfile::TempDir;

#[cfg(windows)]
use std::os::windows::fs::symlink_dir;

fn make_test_repo() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    let git = Command::new("git")
        .args(["init"])
        .current_dir(dir.path())
        .output()
        .unwrap();
    assert!(git.status.success());

    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(dir.path().join("src/main.rs"), "fn main() {}\n").unwrap();

    assert!(
        Command::new("git")
            .args(["add", "."])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );
    assert!(
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
            .unwrap()
            .status
            .success()
    );
    dir
}

#[test]
fn validates_read_paths_before_access() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    let unix_absolute_path = "/etc/passwd";
    let unix_absolute = policy
        .read_file(&repo, unix_absolute_path)
        .expect_err("absolute paths must be denied before file access");
    assert_eq!(
        unix_absolute,
        SecurityError {
            requested: PathBuf::from(unix_absolute_path),
        },
        "an absolute path must produce the security denial result, not an ordinary read error"
    );

    let windows_absolute_path = dir
        .path()
        .parent()
        .unwrap()
        .join("outside-repository-secret");
    std::fs::write(&windows_absolute_path, "secret outside the repository").unwrap();
    let windows_absolute_text = windows_absolute_path.to_string_lossy().into_owned();
    let windows_absolute = policy
        .read_file(&repo, &windows_absolute_text)
        .expect_err("drive-qualified absolute paths must be denied before file access");
    assert_eq!(
        windows_absolute,
        SecurityError {
            requested: PathBuf::from(&windows_absolute_text),
        },
        "a drive-qualified absolute path must produce the security denial result"
    );

    let escaping_path = "../../secret";
    let escaping = policy
        .read_file(&repo, escaping_path)
        .expect_err("paths escaping the repository must be denied before file access");
    assert_eq!(
        escaping,
        SecurityError {
            requested: PathBuf::from(escaping_path),
        },
        "an escaping path must be denied before file access"
    );

    let allowed = policy
        .read_file(&repo, "src/main.rs")
        .expect("relative paths inside the repository should be readable");
    assert_eq!(allowed.content, "fn main() {}\n");
}

#[test]
fn directory_listing_uses_default_entry_limit_and_reports_entry_count() {
    let dir = make_test_repo();
    std::fs::create_dir(dir.path().join("entries")).unwrap();
    for n in 1..=500 {
        std::fs::write(dir.path().join(format!("entries/entry-{n:03}")), "").unwrap();
    }
    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    let bounded = policy
        .list_directory(&repo, "entries")
        .expect("a repository directory should be listable");

    let expected = (1..=100)
        .map(|n| format!("entry-{n:03}"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n[truncated: 100 of 500 entries]";

    assert_eq!(bounded.content, expected);
    assert!(bounded.truncated);
    assert!(!bounded.completeness);
}

#[test]
fn search_results_report_incompleteness_when_match_limit_is_exceeded() {
    let matches = (1..=51)
        .map(|n| format!("src/file-{n:03}.rs:{n}:match"))
        .collect::<Vec<_>>();

    let bounded = bound_matches(&matches);

    let expected = (1..=50)
        .map(|n| format!("src/file-{n:03}.rs:{n}:match"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n[truncated: 50 matches, search incomplete]";
    assert_eq!(bounded.content, expected);
    assert!(bounded.truncated);
    assert!(!bounded.completeness);

    let complete = bound_matches(&matches[..50]);
    assert_eq!(
        complete.content,
        (1..=50)
            .map(|n| format!("src/file-{n:03}.rs:{n}:match"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    assert!(!complete.truncated);
    assert!(complete.completeness);
}

#[test]
fn search_operation_bounds_matches_and_marks_incomplete_results() {
    let dir = make_test_repo();
    for n in 1..=51 {
        std::fs::write(dir.path().join(format!("src/file-{n:03}.rs")), "needle\n").unwrap();
    }
    // These files ensure the operation honors both the requested path and query,
    // rather than merely returning an arbitrary set of 50 lines.
    std::fs::write(dir.path().join("src/not-a-match.rs"), "different\n").unwrap();
    std::fs::write(dir.path().join("outside.rs"), "needle\n").unwrap();

    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    let bounded = policy
        .search(&repo, "src", "needle")
        .expect("searching a repository directory should succeed");

    let expected_matches = (1..=50)
        .map(|n| format!("src/file-{n:03}.rs:1:needle"))
        .collect::<Vec<_>>()
        .join("\n");
    let expected = expected_matches + "\n[truncated: 50 matches, search incomplete]";
    assert_eq!(
        bounded.content, expected,
        "search must return the first 50 actual matches for the requested path and query"
    );
    assert!(bounded.truncated);
    assert!(!bounded.completeness);

    let complete = policy
        .search(&repo, "src", "different")
        .expect("searching for a query with one result should succeed");
    assert_eq!(complete.content, "src/not-a-match.rs:1:different");
    assert!(!complete.truncated);
    assert!(
        complete.completeness,
        "search results must be marked complete when every match fits within the limit"
    );
}

#[test]
fn allows_relative_path_inside_repository() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    assert!(
        SecurityPolicy::new()
            .validate_path(&repo, "src/main.rs")
            .is_ok()
    );
}

#[test]
fn normalizes_dot_and_in_root_parent_segments_before_deny_matching() {
    let dir = make_test_repo();
    // Ensure the raw path contains a real directory that unambiguously matches
    // the **/.aws/** deny pattern, while its normalized target is safe.
    std::fs::create_dir(dir.path().join("src/.aws")).unwrap();
    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    // The raw spelling matches the sensitive-directory pattern at src/.aws,
    // but lexical normalization resolves it to the existing safe file.
    let resolved = policy
        .read_file(&repo, "src/.aws/.././main.rs")
        .expect("a path resolving inside the repository must not be denied for raw segments");
    assert_eq!(resolved.content, "fn main() {}\n");

    assert!(
        policy.validate_path(&repo, "src/../.env").is_err(),
        "a path whose normalized target is deny-listed must remain rejected"
    );
    assert!(
        policy.validate_path(&repo, "src/../../main.rs").is_err(),
        "a path whose normalized target escapes the repository must remain rejected"
    );
}

#[test]
fn rejects_sensitive_paths() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();
    for path in [
        ".git/config",
        ".env",
        ".env.local",
        ".aws/credentials",
        "id_rsa",
    ] {
        assert!(policy.validate_path(&repo, path).is_err(), "{path}");
    }
}

#[cfg(windows)]
#[test]
fn rejects_out_of_repo_symlink() {
    let dir = make_test_repo();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret"), "secret").unwrap();
    symlink_dir(outside.path(), dir.path().join("src/link")).unwrap();
    let repo = GitRepo::open(dir.path()).unwrap();
    assert!(
        SecurityPolicy::new()
            .validate_path(&repo, "src/link")
            .is_err()
    );
}

#[test]
fn bounds_large_file_reads_with_a_machine_readable_marker() {
    let total_bytes = 102_400;
    let byte_limit = 10_240;
    let raw = vec![b'x'; total_bytes];

    let dir = make_test_repo();
    std::fs::write(dir.path().join("src/large.txt"), &raw).unwrap();
    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    let read = policy
        .read_file(&repo, "src/large.txt")
        .expect("the large repository file should be readable");

    let mut expected = "x".repeat(byte_limit);
    expected.push_str("\n[truncated: 10240 of 102400 bytes]");

    assert_eq!(
        read.content, expected,
        "file reads must return the raw-content byte bound and machine-readable marker"
    );
    assert!(read.truncated);
    assert!(!read.completeness, "a truncated file read is incomplete");
}

#[cfg(windows)]
#[test]
fn rejects_external_symlinks_and_allows_internal_symlinks() {
    let repo_dir = make_test_repo();
    let repo = GitRepo::open(repo_dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    let outside_dir = tempfile::tempdir().unwrap();
    std::fs::write(outside_dir.path().join("secret.txt"), "outside").unwrap();
    symlink_dir(outside_dir.path(), repo_dir.path().join("src/link")).unwrap();

    let external_path = "src/link/secret.txt";
    let external_error = policy
        .validate_path(&repo, external_path)
        .expect_err("a symlink resolving outside the repository must be rejected");
    assert_eq!(
        external_error,
        SecurityError {
            requested: PathBuf::from(external_path)
        }
    );

    let internal_target = repo_dir.path().join("src/real");
    std::fs::create_dir(&internal_target).unwrap();
    std::fs::write(internal_target.join("main.rs"), "inside").unwrap();
    symlink_dir(&internal_target, repo_dir.path().join("src/link_inside")).unwrap();

    let internal_path = "src/link_inside/main.rs";
    let resolved = policy
        .validate_path(&repo, internal_path)
        .expect("symlinks resolving within the repository must be allowed");
    assert_eq!(
        resolved,
        internal_target.join("main.rs").canonicalize().unwrap()
    );
}

#[test]
fn denies_sensitive_paths_without_revealing_existence() {
    let dir = make_test_repo();
    let repo = GitRepo::open(dir.path()).unwrap();
    let policy = SecurityPolicy::new();

    let existing_sensitive = [
        (".git/config", "git configuration"),
        (".env", "TOKEN=secret\n"),
        (".env.local", "LOCAL_SECRET=secret\n"),
        ("nested/config/.aws/credentials", "aws secret\n"),
        ("id_rsa", "private key\n"),
        ("nested/keys/id_rsa_backup", "private key\n"),
        ("certificate.pem", "certificate\n"),
        ("nested/config/server.pem", "certificate\n"),
        ("private.key", "private key\n"),
        ("nested/keys/server.key", "private key\n"),
    ];
    for (path, contents) in existing_sensitive {
        let file = dir.path().join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        if path != ".git/config" {
            std::fs::write(&file, contents).unwrap();
        }
        assert!(file.is_file(), "the sensitive fixture must exist: {path}");
    }

    let absent_sensitive = [
        "missing/.git/config",
        ".env.missing",
        ".env.other",
        "missing/nested/.aws/credentials",
        "missing/id_rsa",
        "missing/nested/id_rsa_backup",
        "missing/certificate.pem",
        "missing/nested/server.pem",
        "missing/private.key",
        "missing/nested/server.key",
    ];

    for ((existing, _), nonexistent) in existing_sensitive.iter().zip(absent_sensitive) {
        assert!(!dir.path().join(nonexistent).exists());

        for path in [*existing, nonexistent] {
            assert_eq!(
                policy.validate_path(&repo, path),
                Err(SecurityError {
                    requested: PathBuf::from(path)
                }),
                "deny-listed path must be rejected before filesystem access: {path}"
            );
            assert_eq!(
                policy.read_file(&repo, path),
                Err(SecurityError {
                    requested: PathBuf::from(path)
                }),
                "deny-listed path must be denied before it is read: {path}"
            );
        }
    }
}
