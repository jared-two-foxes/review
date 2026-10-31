use agent_kernel::tools::{Tool, ToolStatus};
use code_agent_runtime::repo::GitRepo;
use code_agent_runtime::security::SecurityPolicy;
use code_agent_runtime::snapshot::resolve_targets;
use code_agent_runtime::target::ReviewTarget;
use code_agent_runtime::tools::SearchTextTool;
use git2::Oid;
use serde_json::json;
use std::process::Command;
use tempfile::TempDir;

fn make_repo_with_searchable_text() -> TempDir {
    let dir = tempfile::tempdir().unwrap();
    assert!(
        Command::new("git")
            .args(["init"])
            .current_dir(dir.path())
            .output()
            .unwrap()
            .status
            .success()
    );

    std::fs::create_dir(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("src/one.txt"),
        "first foo.bar match\nfooXbar must not match\nsecond foo.bar match\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/two.txt"),
        "third foo.bar match\nfourth foo.bar match\n",
    )
    .unwrap();

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
                "search fixtures",
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
fn search_text_returns_literal_structured_matches_with_completeness() {
    let dir = make_repo_with_searchable_text();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::with_matches_limit(2));

    assert_eq!(tool.name(), "search_text");
    tool.validate_arguments(&json!({"query": "foo.bar"}))
        .unwrap();
    assert!(
        tool.validate_arguments(&json!({"query": "foo.bar", "extra": true}))
            .is_err()
    );
    std::fs::write(
        dir.path().join("src/one.txt"),
        "working tree foo.bar match\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/working-only.txt"),
        "working foo.bar match\n",
    )
    .unwrap();

    let result = (&tool as &dyn Tool).execute(&json!({"query": "foo.bar"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(
        result.value["matches"],
        json!([
            {"path": "src/one.txt", "line": 1, "content": "first foo.bar match"},
            {"path": "src/one.txt", "line": 3, "content": "second foo.bar match"}
        ])
    );
    assert_eq!(result.value["completeness"], false);
}

#[test]
fn search_text_bounds_matches_and_reports_incompleteness() {
    let dir = make_repo_with_searchable_text();
    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::with_matches_limit(3));

    std::fs::write(
        dir.path().join("src/one.txt"),
        "working tree foo.bar match\n",
    )
    .unwrap();
    std::fs::write(
        dir.path().join("src/working-only.txt"),
        "working foo.bar match\n",
    )
    .unwrap();

    let result = (&tool as &dyn Tool).execute(&json!({"query": "foo.bar"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(
        result.value["matches"],
        json!([
            {"path": "src/one.txt", "line": 1, "content": "first foo.bar match"},
            {"path": "src/one.txt", "line": 3, "content": "second foo.bar match"},
            {"path": "src/two.txt", "line": 1, "content": "third foo.bar match"}
        ])
    );
    assert_eq!(result.value["completeness"], false);
}

#[test]
fn search_text_ignores_symlink_targets() {
    let dir = tempfile::tempdir().unwrap();
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(dir.path())
            .output()
            .unwrap();
        assert!(output.status.success(), "git {:?}: {:?}", args, output);
        output
    };

    git(&["init"]);
    std::fs::write(dir.path().join("needle.txt"), "ordinary content\n").unwrap();
    // The blob content is a target path containing the query. Install it in
    // the index with Git's symlink mode so the fixture does not need an OS
    // filesystem symlink (and remains portable on Windows).
    std::fs::write(dir.path().join("link.txt"), "needle.txt\n").unwrap();
    git(&["add", "."]);
    let blob_oid = String::from_utf8(git(&["hash-object", "link.txt"]).stdout)
        .unwrap()
        .trim()
        .to_string();
    git(&[
        "update-index",
        "--add",
        "--cacheinfo",
        &format!("120000,{},link.txt", blob_oid),
    ]);
    git(&[
        "-c",
        "user.name=Test",
        "-c",
        "user.email=test@test.com",
        "commit",
        "-m",
        "symlink fixture",
    ]);

    let repo = GitRepo::open(dir.path()).unwrap();
    let (_, head) = resolve_targets(&repo, "HEAD", "HEAD").unwrap();
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::default());

    let result = (&tool as &dyn Tool).execute(&json!({"query": "needle.txt"}));

    assert!(matches!(result.status, ToolStatus::Succeeded));
    assert_eq!(result.value["matches"], json!([]));
    for target in [ReviewTarget::parse(&GitRepo::open(dir.path()).unwrap(), "HEAD").unwrap(), ReviewTarget::Index] {
        let tool = SearchTextTool::new(GitRepo::open(dir.path()).unwrap(), target, SecurityPolicy::default());
        let result = tool.execute(&json!({"query": "needle.txt", "path": "link.txt", "offset": 1, "limit": 1}));
        assert!(matches!(result.status, ToolStatus::Succeeded));
        assert_eq!(result.value["matches"], json!([]));
    }
}

#[test]
fn search_text_returns_failed_result_when_head_tree_cannot_be_resolved() {
    let dir = make_repo_with_searchable_text();
    let repo = GitRepo::open(dir.path()).unwrap();
    let tool = SearchTextTool::new(
        repo,
        ReviewTarget::Commit(Oid::zero()),
        SecurityPolicy::default(),
    );

    let result = (&tool as &dyn Tool).execute(&json!({"query": "foo.bar"}));

    assert!(matches!(result.status, ToolStatus::Failed));
    assert_eq!(
        result.value,
        json!({"error": "failed to resolve head tree"})
    );
}

fn search_at(dir: &TempDir, target: &str, args: serde_json::Value, cap: usize) -> serde_json::Value {
    let repo = GitRepo::open(dir.path()).unwrap();
    let head = ReviewTarget::parse(&repo, target).unwrap();
    let label = head.label(&repo);
    let tool = SearchTextTool::new(repo, head, SecurityPolicy::with_matches_limit(cap));
    let result = tool.execute(&args);
    assert!(matches!(result.status, ToolStatus::Succeeded), "{target}: {args}: {:?}", result.value);
    assert_eq!(result.value["observed_head"], label);
    result.value
}

fn match_locations(value: &serde_json::Value) -> Vec<(String, u64)> {
    value["matches"].as_array().unwrap().iter().map(|item| {
        (item["path"].as_str().unwrap().replace('\\', "/"), item["line"].as_u64().unwrap())
    }).collect()
}

#[test]
fn search_text_validates_optional_scope_and_advertises_schema() {
    let dir = make_repo_with_searchable_text();
    let tool = SearchTextTool::new(GitRepo::open(dir.path()).unwrap(), ReviewTarget::Index, SecurityPolicy::default());
    let schema = tool.description().input_schema;
    let validator = jsonschema::validator_for(&schema).unwrap();
    assert_eq!(schema["required"], json!(["query"]));
    assert_eq!(schema["additionalProperties"], false);
    assert_eq!(schema["properties"]["path"]["type"], "string");
    for key in ["offset", "limit"] {
        assert_eq!(schema["properties"][key]["type"], "integer");
        assert_eq!(schema["properties"][key]["minimum"], 1);
        assert_eq!(schema["properties"][key]["maximum"], json!(u64::MAX));
        for bad in [json!(0), json!(-1), json!(1.5), json!("1"), json!(true), json!(null), json!([]), json!({}), serde_json::from_str("18446744073709551616").unwrap()] {
            let mut args = json!({"query": "foo.bar"});
            args[key] = bad;
            assert!(!validator.is_valid(&args), "schema accepted {args}");
            assert!(tool.validate_arguments(&args).is_err(), "{args}");
            assert!(matches!(tool.execute(&args).status, ToolStatus::Failed), "{args}");
        }
    }
    for args in [json!(null), json!([]), json!("query"), json!({}), json!({"query": 1}), json!({"query": "x", "extra": true}), json!({"query": "x", "path": null}), json!({"query": "x", "path": 1})] {
        assert!(!validator.is_valid(&args), "schema accepted {args}");
        assert!(tool.validate_arguments(&args).is_err());
        assert!(matches!(tool.execute(&args).status, ToolStatus::Failed));
    }
    for path in ["", ".", "..", "../outside", "src/../one.txt", "/absolute", "C:/absolute", "src\\one.txt", "src//one.txt", "./src/one.txt", "src/"] {
        let args = json!({"query": "x", "path": path});
        assert!(!validator.is_valid(&args), "schema accepted {args}");
        assert!(tool.validate_arguments(&args).is_err(), "{path}");
        assert!(matches!(tool.execute(&args).status, ToolStatus::Failed));
    }
    for args in [json!({"query": "x"}), json!({"query": "x", "path": "src/one.txt"}), json!({"query": "x", "offset": 2}), json!({"query": "x", "limit": 2}), json!({"query": "x", "path": "src/one.txt", "offset": u64::MAX, "limit": u64::MAX})] {
        assert!(validator.is_valid(&args), "schema rejected {args}");
        tool.validate_arguments(&args).unwrap();
    }
    for path in [".hidden", "src/.hidden.txt", "...", "src/...", "src/a b.txt", "src/λ.txt", ".\n", "src/..\n"] {
        let args = json!({"query": "x", "path": path});
        assert!(validator.is_valid(&args), "schema rejected {args}");
        tool.validate_arguments(&args).unwrap();
    }
}

#[test]
fn search_text_line_ranges_apply_to_all_targets() {
    let dir = make_repo_with_searchable_text();
    let cases = [
        (json!({"query": "foo.bar", "path": "src/one.txt"}), vec![("src/one.txt", 1), ("src/one.txt", 3)]),
        (json!({"query": "foo.bar", "offset": 2}), vec![("src/one.txt", 3), ("src/two.txt", 2)]),
        (json!({"query": "foo.bar", "limit": 1}), vec![("src/one.txt", 1), ("src/two.txt", 1)]),
        (json!({"query": "foo.bar", "path": "src/one.txt", "offset": 2, "limit": 1}), vec![]),
        (json!({"query": "foo.bar", "path": "src/one.txt", "offset": 2, "limit": 2}), vec![("src/one.txt", 3)]),
        (json!({"query": "foo.bar", "offset": 2, "limit": 1}), vec![("src/two.txt", 2)]),
        (json!({"query": "foo.bar", "offset": 4}), vec![]),
        (json!({"query": "foo.bar", "offset": u64::MAX, "limit": u64::MAX}), vec![]),
        (json!({"query": "foo.bar", "path": "src/one.txt", "offset": 3, "limit": u64::MAX}), vec![("src/one.txt", 3)]),
    ];
    for target in ["HEAD", ":staged", ":working"] {
        for (args, expected) in &cases {
            let result = search_at(&dir, target, args.clone(), 50);
            let expected: Vec<_> = expected.iter().map(|(p, l)| (p.to_string(), *l)).collect();
            assert_eq!(match_locations(&result), expected, "{target}: {args}");
            assert_eq!(result["completeness"], true);
        }
    }
}

#[test]
fn search_text_filters_scope_before_match_cap() {
    let dir = make_repo_with_searchable_text();
    for target in ["HEAD", ":staged", ":working"] {
        let args = json!({"query": "foo.bar", "path": "src/two.txt", "offset": 2, "limit": 1});
        let result = search_at(&dir, target, args.clone(), 1);
        assert_eq!(match_locations(&result), vec![("src/two.txt".to_string(), 2)]);
        assert_eq!(result["completeness"], true);
        let result = search_at(&dir, target, args, 0);
        assert_eq!(result["matches"], json!([]));
        assert_eq!(result["completeness"], false);
        let result = search_at(&dir, target, json!({"query": "foo.bar", "path": "src/one.txt"}), 1);
        assert_eq!(match_locations(&result), vec![("src/one.txt".to_string(), 1)]);
        assert_eq!(result["completeness"], false);
        let result = search_at(&dir, target, json!({"query": "foo.bar", "offset": 4}), 0);
        assert_eq!(result["completeness"], true);
    }
}

#[test]
fn search_text_scoped_queries_observe_selected_target() {
    let dir = make_repo_with_searchable_text();
    std::fs::write(dir.path().join("src/one.txt"), "no match\nfoo.bar staged\n").unwrap();
    let output = Command::new("git").args(["add", "src/one.txt"]).current_dir(dir.path()).output().unwrap();
    assert!(output.status.success());
    std::fs::write(dir.path().join("src/one.txt"), "no match\nno match\nfoo.bar working\n").unwrap();
    for (target, expected) in [("HEAD", 0), (":staged", 1), (":working", 0)] {
        let result = search_at(&dir, target, json!({"query": "foo.bar", "path": "src/one.txt", "offset": 2, "limit": 1}), 50);
        assert_eq!(result["matches"].as_array().unwrap().len(), expected);
        if expected > 0 {
            assert_eq!(result["matches"][0]["content"], "foo.bar staged");
        }
    }
    std::fs::remove_file(dir.path().join("src/one.txt")).unwrap();
    let result = search_at(&dir, "HEAD", json!({"query": "foo.bar", "path": "src/one.txt", "offset": 3}), 50);
    assert_eq!(match_locations(&result), vec![("src/one.txt".to_string(), 3)]);
}

#[test]
fn search_text_handles_line_endings_empty_files_and_unicode() {
    let dir = make_repo_with_searchable_text();
    let text = "foo.bar first\r\n\r\nfoo.bar λ\nfoo.bar final";
    std::fs::write(dir.path().join("src/lines.txt"), text).unwrap();
    std::fs::write(dir.path().join("src/empty.txt"), "").unwrap();
    let output = Command::new("git").args(["-c", "core.autocrlf=false", "add", "."]).current_dir(dir.path()).output().unwrap();
    assert!(output.status.success());
    let output = Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "-m", "line fixtures"]).current_dir(dir.path()).output().unwrap();
    assert!(output.status.success());
    for target in ["HEAD", ":staged", ":working"] {
        for (offset, limit, expected) in [(2, 1, vec![]), (3, 1, vec![3]), (3, 2, vec![3, 4]), (4, u64::MAX, vec![4]), (5, 1, vec![])] {
            let result = search_at(&dir, target, json!({"query": "foo.bar", "path": "src/lines.txt", "offset": offset, "limit": limit}), 50);
            let lines: Vec<_> = match_locations(&result).into_iter().map(|(_, line)| line).collect();
            assert_eq!(lines, expected, "{target}: offset={offset} limit={limit}");
        }
        let result = search_at(&dir, target, json!({"query": "λ", "path": "src/lines.txt", "offset": 3, "limit": 1}), 50);
        assert_eq!(match_locations(&result), vec![("src/lines.txt".to_string(), 3)]);
        let result = search_at(&dir, target, json!({"query": "", "path": "src/empty.txt", "offset": 1, "limit": u64::MAX}), 50);
        assert_eq!(result["matches"], json!([]));
    }
}

#[test]
fn search_text_path_is_an_exact_filter_not_a_new_read_interface() {
    let dir = make_repo_with_searchable_text();
    for target in ["HEAD", ":staged", ":working"] {
        for path in ["missing.txt", "src", "src/one", "one.txt"] {
            let result = search_at(&dir, target, json!({"query": "foo.bar", "path": path}), 50);
            assert_eq!(result["matches"], json!([]));
            assert_eq!(result["completeness"], true);
        }
    }
    std::fs::write(dir.path().join(".gitignore"), "ignored.txt\n").unwrap();
    std::fs::write(dir.path().join("ignored.txt"), "foo.bar\n").unwrap();
    let result = search_at(&dir, ":working", json!({"query": "foo.bar", "path": "ignored.txt"}), 50);
    assert_eq!(result["matches"], json!([]));
}

#[test]
fn search_text_denies_sensitive_path_scopes() {
    let dir = make_repo_with_searchable_text();
    std::fs::write(dir.path().join(".env"), "foo.bar secret\n").unwrap();
    for target in ["HEAD", ":staged", ":working"] {
        let repo = GitRepo::open(dir.path()).unwrap();
        let head = ReviewTarget::parse(&repo, target).unwrap();
        let tool = SearchTextTool::new(repo, head, SecurityPolicy::default());
        for path in [".env", ".git/config", "src/private.key"] {
            let result = tool.execute(&json!({"query": "foo.bar", "path": path, "offset": 1, "limit": 1}));
            assert!(matches!(result.status, ToolStatus::Denied));
            assert!(result.value.get("matches").is_none());
        }
    }
}

#[cfg(unix)]
#[test]
fn search_text_scope_does_not_alias_non_utf8_filenames() {
    use std::os::unix::ffi::OsStringExt;

    let dir = make_repo_with_searchable_text();
    let invalid = std::ffi::OsString::from_vec(b"src/\xff.txt".to_vec());
    let invalid_missing = std::ffi::OsString::from_vec(b"src/\xfe-missing.txt".to_vec());
    std::fs::write(dir.path().join(invalid), "foo.bar wrong file\n").unwrap();
    std::fs::write(dir.path().join(invalid_missing), "foo.bar wrong missing file\n").unwrap();
    std::fs::write(dir.path().join("src/�.txt"), "foo.bar correct file\n").unwrap();
    let output = Command::new("git").args(["add", "."]).current_dir(dir.path()).output().unwrap();
    assert!(output.status.success());
    let output = Command::new("git").args(["-c", "user.name=Test", "-c", "user.email=test@test.com", "commit", "-m", "filename fixtures"]).current_dir(dir.path()).output().unwrap();
    assert!(output.status.success());
    for target in ["HEAD", ":staged", ":working"] {
        let result = search_at(&dir, target, json!({"query": "foo.bar", "path": "src/�.txt", "offset": 1, "limit": 1}), 50);
        assert_eq!(result["matches"].as_array().unwrap().len(), 1, "{target}");
        assert_eq!(result["matches"][0]["content"].as_str().unwrap().trim_end(), "foo.bar correct file");
        let result = search_at(&dir, target, json!({"query": "foo.bar", "path": "src/�-missing.txt"}), 50);
        assert_eq!(result["matches"], json!([]), "{target}");
    }
}

#[cfg(unix)]
#[test]
fn search_text_working_scope_preserves_literal_backslash_filenames() {
    let dir = make_repo_with_searchable_text();
    std::fs::write(dir.path().join("src\\one.txt"), "foo.bar wrong file\n").unwrap();
    std::fs::write(dir.path().join("src\\missing.txt"), "foo.bar wrong file\n").unwrap();
    std::fs::create_dir(dir.path().join("\\src")).unwrap();
    std::fs::write(dir.path().join("\\src/one.txt"), "foo.bar wrong directory\n").unwrap();
    let result = search_at(&dir, ":working", json!({"query": "foo.bar", "path": "src/one.txt", "offset": 1, "limit": 1}), 50);
    assert_eq!(result["matches"].as_array().unwrap().len(), 1);
    assert_eq!(result["matches"][0]["path"], "src/one.txt");
    assert_eq!(result["matches"][0]["content"].as_str().unwrap().trim_end(), "first foo.bar match");
    let result = search_at(&dir, ":working", json!({"query": "foo.bar", "path": "src/missing.txt"}), 50);
    assert_eq!(result["matches"], json!([]));
}

#[cfg(unix)]
#[test]
fn search_text_working_scope_does_not_follow_symlinks() {
    let dir = make_repo_with_searchable_text();
    let outside = tempfile::tempdir().unwrap();
    std::fs::write(outside.path().join("secret.txt"), "foo.bar secret\n").unwrap();
    std::os::unix::fs::symlink(outside.path().join("secret.txt"), dir.path().join("external.txt")).unwrap();
    std::os::unix::fs::symlink("src/one.txt", dir.path().join("internal.txt")).unwrap();
    for path in ["external.txt", "internal.txt"] {
        let result = search_at(&dir, ":working", json!({"query": "foo.bar", "path": path, "offset": 1, "limit": 1}), 50);
        assert_eq!(result["matches"], json!([]));
    }
}
