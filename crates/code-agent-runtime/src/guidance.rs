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

    let mut partial = PathBuf::new();
    for segment in segments {
        partial.push(segment);
        directories.push(repository_path.join(&partial));
    }

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
