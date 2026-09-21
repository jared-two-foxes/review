use crate::repo::GitRepo;
use crate::security::SecurityPolicy;
use std::path::{Path, PathBuf};

const MAX_GUIDANCE_BYTES: usize = 65_536;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuidanceKind {
    Readme,
    Agents,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuidanceDocument {
    pub kind: GuidanceKind,
    pub path: PathBuf,
    pub content: String,
}

pub fn collect_guidance_documents(
    repository_path: &Path,
    focus_path: Option<&str>,
) -> Vec<GuidanceDocument> {
    if let Ok(repo) = GitRepo::open(repository_path) {
        let policy = SecurityPolicy::with_bytes_limit(MAX_GUIDANCE_BYTES);
        collect_guidance_documents_with_repo(&repo, &policy, focus_path)
    } else {
        collect_guidance_documents_from_filesystem(repository_path, focus_path)
    }
}

fn collect_guidance_documents_with_repo(
    repo: &GitRepo,
    policy: &SecurityPolicy,
    focus_path: Option<&str>,
) -> Vec<GuidanceDocument> {
    let mut documents = vec![];

    if let Some(readme) = read_readme(repo, policy) {
        documents.push(GuidanceDocument {
            kind: GuidanceKind::Readme,
            path: readme.0,
            content: readme.1,
        });
    }

    for (path, content) in read_cascading_agents(repo, policy, focus_path) {
        documents.push(GuidanceDocument {
            kind: GuidanceKind::Agents,
            path,
            content,
        });
    }

    documents
}

fn collect_guidance_documents_from_filesystem(
    repository_path: &Path,
    focus_path: Option<&str>,
) -> Vec<GuidanceDocument> {
    let mut documents = vec![];

    if let Some(readme) = read_readme_from_filesystem(repository_path) {
        documents.push(GuidanceDocument {
            kind: GuidanceKind::Readme,
            path: readme.0,
            content: readme.1,
        });
    }

    for (path, content) in read_cascading_agents_from_filesystem(repository_path, focus_path) {
        documents.push(GuidanceDocument {
            kind: GuidanceKind::Agents,
            path,
            content,
        });
    }

    documents
}

fn read_readme(repo: &GitRepo, policy: &SecurityPolicy) -> Option<(PathBuf, String)> {
    for candidate in ["README.md", "README", "readme.md", "readme"] {
        let path = repo.root().join(candidate);
        if let Some(content) = read_file_limited(repo, policy, candidate) {
            return Some((path, content));
        }
    }
    None
}

fn read_readme_from_filesystem(repository_path: &Path) -> Option<(PathBuf, String)> {
    for candidate in ["README.md", "README", "readme.md", "readme"] {
        let path = repository_path.join(candidate);
        if let Some(content) = read_file_limited_from_filesystem(&path) {
            return Some((path, content));
        }
    }
    None
}

fn read_cascading_agents(
    repo: &GitRepo,
    policy: &SecurityPolicy,
    focus_path: Option<&str>,
) -> Vec<(PathBuf, String)> {
    let mut results = vec![];
    for directory in cascading_directories(repo, policy, focus_path) {
        if let Some(found) = read_agents_in_directory(repo, policy, &directory) {
            results.push(found);
        }
    }
    results
}

fn read_cascading_agents_from_filesystem(
    repository_path: &Path,
    focus_path: Option<&str>,
) -> Vec<(PathBuf, String)> {
    let mut results = vec![];
    for directory in cascading_directories_from_root(repository_path, focus_path) {
        if let Some(found) = read_agents_in_directory_from_filesystem(&directory) {
            results.push(found);
        }
    }
    results
}

fn cascading_directories(
    repo: &GitRepo,
    policy: &SecurityPolicy,
    focus_path: Option<&str>,
) -> Vec<PathBuf> {
    let repository_path = repo.root();
    cascading_directories_with_filter(repository_path, focus_path, |directory| {
        read_agents_in_directory(repo, policy, directory).map(|(_, content)| content)
    })
}

fn cascading_directories_from_root(
    repository_path: &Path,
    focus_path: Option<&str>,
) -> Vec<PathBuf> {
    cascading_directories_with_filter(repository_path, focus_path, |directory| {
        read_agents_in_directory_from_filesystem(directory).map(|(_, content)| content)
    })
}

fn cascading_directories_with_filter<F>(
    repository_path: &Path,
    focus_path: Option<&str>,
    mut parent_guidance_for_directory: F,
) -> Vec<PathBuf>
where
    F: FnMut(&Path) -> Option<String>,
{
    let mut directories = vec![repository_path.to_path_buf()];
    let Some(focus_path) = focus_path else {
        return directories;
    };

    if focus_path.trim().is_empty() {
        return directories;
    }

    let mut segments: Vec<PathBuf> = vec![];
    for component in Path::new(focus_path).components() {
        match component {
            std::path::Component::Normal(part) => segments.push(PathBuf::from(part)),
            std::path::Component::ParentDir => {
                segments.pop();
            }
            std::path::Component::CurDir => {}
            std::path::Component::RootDir | std::path::Component::Prefix(_) => {
                return directories;
            }
        }
    }

    if !focus_path.ends_with('/') && !segments.is_empty() {
        segments.pop();
    }

    if segments.is_empty() {
        return directories;
    }

    let mut parent_agents = parent_guidance_for_directory(repository_path);
    let mut partial = PathBuf::new();
    for segment in segments {
        if let Some(content) = &parent_agents
            && !guidance_recommends_descending(content, &segment)
        {
            break;
        }
        partial.push(segment);
        let next_directory = repository_path.join(&partial);
        directories.push(next_directory.clone());
        parent_agents = parent_guidance_for_directory(&next_directory);
    }

    directories
}

fn guidance_recommends_descending(parent_guidance: &str, child_segment: &Path) -> bool {
    let guidance = parent_guidance.to_ascii_lowercase();
    let child = child_segment.to_string_lossy().to_ascii_lowercase();
    if child.is_empty() {
        return false;
    }

    if guidance.contains("all directories")
        || guidance.contains("all files")
        || guidance.contains("entire repository")
        || guidance.contains("entire repo")
        || guidance.contains("any path")
    {
        return true;
    }

    guidance.contains(&child) || guidance.contains(&format!("{child}/"))
}

fn read_agents_in_directory(
    repo: &GitRepo,
    policy: &SecurityPolicy,
    directory: &Path,
) -> Option<(PathBuf, String)> {
    let relative_directory = directory.strip_prefix(repo.root()).ok()?;
    for candidate in ["AGENTS.md", "agents.md"] {
        let path = directory.join(candidate);
        let relative_path = relative_directory.join(candidate);
        let Some(relative_path) = relative_path.to_str() else {
            continue;
        };
        if let Some(content) = read_file_limited(repo, policy, relative_path) {
            return Some((path, content));
        }
    }
    None
}

fn read_agents_in_directory_from_filesystem(directory: &Path) -> Option<(PathBuf, String)> {
    for candidate in ["AGENTS.md", "agents.md"] {
        let path = directory.join(candidate);
        if let Some(content) = read_file_limited_from_filesystem(&path) {
            return Some((path, content));
        }
    }
    None
}

fn read_file_limited(repo: &GitRepo, policy: &SecurityPolicy, path: &str) -> Option<String> {
    policy
        .read_file(repo, path)
        .ok()
        .map(|bounded| bounded.content)
}

fn read_file_limited_from_filesystem(path: &Path) -> Option<String> {
    let bytes = std::fs::read(path).ok()?;
    let limited = if bytes.len() > MAX_GUIDANCE_BYTES {
        &bytes[..MAX_GUIDANCE_BYTES]
    } else {
        &bytes[..]
    };
    Some(String::from_utf8_lossy(limited).into_owned())
}
