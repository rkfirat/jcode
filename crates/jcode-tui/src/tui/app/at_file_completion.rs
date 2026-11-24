//! `@path` file mention completion for the composer.
//!
//! Typing `@` at the start of input or after whitespace opens the regular
//! suggestion palette with files and directories under the working directory.
//! Accepting a suggestion replaces only the `@token`, so prose around it is
//! kept. Gitignored files are listed on purpose (vaults often ignore the most
//! relevant notes); heavy build/VCS directories are skipped.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

const MAX_ENTRIES: usize = 20_000;
const MAX_DEPTH: usize = 12;
const CACHE_TTL: Duration = Duration::from_secs(5);
const SKIP_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "target",
    ".next",
    ".venv",
    "venv",
    "__pycache__",
    ".cache",
    "dist",
    ".obsidian",
    ".trash",
];

pub(super) const FILE_HELP: &str = "file";
pub(super) const DIR_HELP: &str = "dir";

/// Span of the `@token` that ends at `cursor`, including the `@`.
pub(super) fn active_at_token(input: &str, cursor: usize) -> Option<(usize, usize)> {
    let cursor = cursor.min(input.len());
    if !input.is_char_boundary(cursor) {
        return None;
    }
    let before = &input[..cursor];
    let start = before
        .char_indices()
        .rev()
        .find(|(_, c)| c.is_whitespace())
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let token = &before[start..];
    if !token.starts_with('@') {
        return None;
    }
    // `foo@bar` (emails) never reaches here because start is after whitespace.
    Some((start, cursor))
}

struct Cache {
    root: PathBuf,
    at: Instant,
    entries: Vec<(String, bool)>,
}

static CACHE: Mutex<Option<Cache>> = Mutex::new(None);

fn walk(root: &Path) -> Vec<(String, bool)> {
    let mut out = Vec::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut children: Vec<_> = read.flatten().collect();
        children.sort_by_key(|e| e.file_name());
        for entry in children {
            if out.len() >= MAX_ENTRIES {
                return out;
            }
            let name = entry.file_name();
            let name = name.to_string_lossy();
            let Ok(ft) = entry.file_type() else { continue };
            let is_dir = ft.is_dir();
            if is_dir && SKIP_DIRS.contains(&name.as_ref()) {
                continue;
            }
            if name == ".DS_Store" {
                continue;
            }
            let path = entry.path();
            let Ok(rel) = path.strip_prefix(root) else { continue };
            let mut rel = rel.to_string_lossy().into_owned();
            if is_dir {
                rel.push('/');
                if depth + 1 < MAX_DEPTH {
                    stack.push((path, depth + 1));
                }
            }
            out.push((rel, is_dir));
        }
    }
    out
}

fn entries_for(root: &Path) -> Vec<(String, bool)> {
    let mut guard = CACHE.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(cache) = guard.as_ref()
        && cache.root == root
        && cache.at.elapsed() < CACHE_TTL
    {
        return cache.entries.clone();
    }
    let entries = walk(root);
    *guard = Some(Cache {
        root: root.to_path_buf(),
        at: Instant::now(),
        entries: entries.clone(),
    });
    entries
}

/// Ranked `@path` suggestions for the typed token (which includes the `@`).
pub(super) fn suggestions(token: &str) -> Vec<(String, &'static str)> {
    let query = token.trim_start_matches('@').to_lowercase();
    let Ok(root) = std::env::current_dir() else {
        return Vec::new();
    };
    let entries = entries_for(&root);

    let mut scored: Vec<(u8, i32, String, bool)> = Vec::new();
    for (rel, is_dir) in entries {
        let lower = rel.to_lowercase();
        if query.is_empty() {
            // Top level only, so `@` alone shows a tidy listing.
            let top = rel.trim_end_matches('/');
            if !top.contains('/') {
                scored.push((1, 0, rel, is_dir));
            }
            continue;
        }
        let file_name = lower
            .trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        if lower.starts_with(&query) {
            scored.push((3, 0, rel, is_dir));
        } else if file_name.starts_with(&query) {
            scored.push((2, 0, rel, is_dir));
        } else if let Some(score) = crate::tui::fuzzy::fuzzy_score(&query, &lower) {
            scored.push((0, score, rel, is_dir));
        }
    }
    scored.sort_by(|a, b| {
        b.0.cmp(&a.0)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| a.2.len().cmp(&b.2.len()))
            .then_with(|| a.2.cmp(&b.2))
    });
    scored.truncate(200);
    scored
        .into_iter()
        .map(|(_, _, rel, is_dir)| {
            (
                format!("@{}", rel),
                if is_dir { DIR_HELP } else { FILE_HELP },
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn token_detection() {
        assert_eq!(active_at_token("@src", 4), Some((0, 4)));
        assert_eq!(active_at_token("bak @src/ma", 11), Some((4, 11)));
        assert_eq!(active_at_token("mail a@b.com", 12), None);
        assert_eq!(active_at_token("@src bitti", 10), None);
        assert_eq!(active_at_token("selam", 5), None);
    }
}
