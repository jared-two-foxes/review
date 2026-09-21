use std::fs;
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

pub fn collect_guidance_documents(repository_path: &Path, focus_path: Option<&str>) -> Vec<GuidanceDocument> {
    let mut documents = vec![];

    if let Some(readme) = read_readme(repository_path) {
        documents.push(GuidanceDocument {
            kind: GuidanceKind::Readme,
            path: readme.0,
            content: readme.1,
        });
    }

    for (path, content) in read_cascading_agents(repository_path, focus_path) {
        documents.push(GuidanceDocument {
            kind: GuidanceKind::Agents,
            path,
            content,
        });
    }

    documents
}

fn read_readme(repository_path: &Path) -> Option<(PathBuf, String)> {
    for candidate in ["README.md", "README", "readme.md", "readme"] {
        let path = repository_path.join(candidate);
        if let Some(content) = read_file_limited(&path) {
            return Some((path, content));
        }
    }
    None
}

fn read_cascading_agents(repository_path: &Path, focus_path: Option<&str>) -> Vec<(PathBuf, String)> {
    let mut results = vec![];
    for directory in cascading_directories(repository_path, focus_path) {
        if let Some(found) = read_agents_in_directory(&directory) {
            results.push(found);
        }
    }
    results
}

fn cascading_directories(repository_path: &Path, focus_path: Option<&str>) -> Vec<PathBuf> {
    let mut directories = vec![repository_path.to_path_buf()];
    let Some(focus_path) = focus_path else {
        return directories;
    };

    if focus_path.trim().is_empty() {
        return directories;
    }

    let joined = repository_path.join(focus_path);
    let focus_directory = if joined.is_dir() {
        Some(joined)
    } else {
        joined.parent().map(Path::to_path_buf)
    };

    let Some(mut current) = focus_directory else {
        return directories;
    };

    let Ok(repository_canonical) = repository_path.canonicalize() else {
        return directories;
    };

    let mut stack = vec![];
    while let Ok(canonical) = current.canonicalize() {
        if !canonical.starts_with(&repository_canonical) {
            break;
        }
        if canonical == repository_canonical {
            break;
        }
        stack.push(canonical);
        let Some(parent) = current.parent() else {
            break;
        };
        current = parent.to_path_buf();
    }
    stack.reverse();
    directories.extend(stack);
    directories
}

fn read_agents_in_directory(directory: &Path) -> Option<(PathBuf, String)> {
    for candidate in ["AGENTS.md", "agents.md"] {
        let path = directory.join(candidate);
        if let Some(content) = read_file_limited(&path) {
            return Some((path, content));
        }
    }
    None
}

fn read_file_limited(path: &Path) -> Option<String> {
    let bytes = fs::read(path).ok()?;
    let limited = if bytes.len() > MAX_GUIDANCE_BYTES {
        &bytes[..MAX_GUIDANCE_BYTES]
    } else {
        &bytes[..]
    };
    Some(String::from_utf8_lossy(limited).into_owned())
}
