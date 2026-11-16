//! Keeps the mdBook in `docs/book` in sync with `README.md`, which stays the single source of the text.
//!
//! mdBook only warns about an `{{#include}}` with a missing anchor, and a README section that no chapter
//! includes simply vanishes from the site, so these tests fail instead.

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn book_src() -> PathBuf {
    root().join("docs/book/src")
}

/// All chapter files (`*.md`) in the book source folder, sorted.
fn chapters() -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = fs::read_dir(book_src())
        .expect("docs/book/src exists")
        .map(|e| e.expect("directory entry").path())
        .filter(|p| p.extension().is_some_and(|e| e == "md"))
        .collect();
    files.sort();
    files
}

/// Every `{{#include <path>:<anchor>}}` of a chapter as `(path, anchor)`.
fn includes(text: &str) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("{{#include ") {
        let after = &rest[start + "{{#include ".len()..];
        let Some(end) = after.find("}}") else { break };
        let spec = after[..end].trim();
        if let Some((path, anchor)) = spec.rsplit_once(':') {
            found.push((path.to_string(), anchor.to_string()));
        }
        rest = &after[end..];
    }
    found
}

/// Line range `(start, end)` (the lines between the anchor comments) of `anchor` in `lines`.
fn anchor_range(lines: &[&str], anchor: &str) -> Option<(usize, usize)> {
    let open = format!("ANCHOR: {anchor} ");
    let close = format!("ANCHOR_END: {anchor} ");
    let starts: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains(&open))
        .map(|(i, _)| i)
        .collect();
    let ends: Vec<usize> = lines
        .iter()
        .enumerate()
        .filter(|(_, l)| l.contains(&close))
        .map(|(i, _)| i)
        .collect();
    match (starts.as_slice(), ends.as_slice()) {
        ([s], [e]) if s < e => Some((*s, *e)),
        _ => None,
    }
}

/// The README anchors that some chapter includes, as `(start, end)` line ranges.
fn included_readme_ranges(readme: &[&str]) -> Vec<(usize, usize)> {
    let mut ranges = Vec::new();
    for chapter in chapters() {
        let text = fs::read_to_string(&chapter).expect("chapter is readable");
        for (path, anchor) in includes(&text) {
            if path.ends_with("README.md")
                && let Some(range) = anchor_range(readme, &anchor)
            {
                ranges.push(range);
            }
        }
    }
    ranges
}

fn readme_text() -> String {
    fs::read_to_string(root().join("README.md")).expect("README.md is readable")
}

#[test]
fn summary_entries_exist() {
    let summary = fs::read_to_string(book_src().join("SUMMARY.md")).expect("SUMMARY.md exists");
    let mut missing = Vec::new();
    for part in summary.split("](").skip(1) {
        let Some((target, _)) = part.split_once(')') else {
            continue;
        };
        if !book_src().join(target).is_file() {
            missing.push(target.to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "SUMMARY.md links to missing chapters: {missing:?}"
    );
}

#[test]
fn includes_resolve() {
    let mut problems = Vec::new();
    for chapter in chapters() {
        let name = chapter.file_name().unwrap_or_default().to_string_lossy().into_owned();
        let text = fs::read_to_string(&chapter).expect("chapter is readable");
        for (path, anchor) in includes(&text) {
            let target = chapter.parent().unwrap_or(Path::new(".")).join(&path);
            let Ok(content) = fs::read_to_string(&target) else {
                problems.push(format!("{name}: {path} does not exist (anchor {anchor})"));
                continue;
            };
            let lines: Vec<&str> = content.lines().collect();
            if anchor_range(&lines, &anchor).is_none() {
                problems.push(format!(
                    "{name}: {path} has no single ANCHOR/ANCHOR_END pair named {anchor}"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "broken includes:\n{}", problems.join("\n"));
}

#[test]
fn every_readme_section_is_in_the_book() {
    let text = readme_text();
    let readme: Vec<&str> = text.lines().collect();
    let ranges = included_readme_ranges(&readme);
    let mut missing = Vec::new();
    for (i, line) in readme.iter().enumerate() {
        if !line.starts_with("## ") {
            continue;
        }
        let inside = ranges.iter().any(|&(s, e)| s < i && i < e);
        // The chapter title replaces the README heading, so an anchor may start right below it.
        let next = readme
            .iter()
            .skip(i + 1)
            .find(|l| !l.trim().is_empty())
            .copied()
            .unwrap_or("");
        let heads_range = ranges.iter().any(|&(s, _)| readme.get(s).is_some_and(|l| *l == next));
        if !inside && !heads_range {
            missing.push((*line).to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "README sections that no book chapter includes: {missing:?}"
    );
}

#[test]
fn included_readme_links_are_absolute() {
    let text = readme_text();
    let readme: Vec<&str> = text.lines().collect();
    let mut bad = Vec::new();
    for (start, end) in included_readme_ranges(&readme) {
        for line in readme.iter().take(end).skip(start) {
            for part in line.split("](").skip(1) {
                let Some((target, _)) = part.split_once(')') else {
                    continue;
                };
                let ok = ["http://", "https://", "mailto:", "#"]
                    .iter()
                    .any(|p| target.starts_with(p));
                if !ok {
                    bad.push(target.to_string());
                }
            }
        }
    }
    assert!(
        bad.is_empty(),
        "relative links in README parts that the book includes (404 on the site): {bad:?}"
    );
}
