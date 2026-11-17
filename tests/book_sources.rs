//! Keeps the mdBook in `docs/book` in sync with `README.md`, which stays the single source of the text.
//!
//! mdBook only warns about an `{{#include}}` with a missing anchor, and a README section that no chapter
//! includes simply vanishes from the site, so these tests fail instead.

#![expect(clippy::expect_used, reason = "a failing expect is a failing test")]
#![expect(clippy::indexing_slicing, reason = "a failing index is a failing test")]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

fn book_src() -> PathBuf {
    root().join("docs/book/src")
}

/// All chapter files (`*.md`) below the book source folder, sorted.
fn chapters() -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut stack = vec![book_src()];
    while let Some(dir) = stack.pop() {
        for path in fs::read_dir(dir)
            .expect("docs/book/src exists")
            .map(|e| e.expect("directory entry").path())
        {
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "md") {
                files.push(path);
            }
        }
    }
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
        match spec.rsplit_once(':') {
            Some((path, anchor)) if !path.is_empty() && !anchor.contains(['/', '\\']) => {
                found.push((path.to_string(), anchor.to_string()));
            }
            // no `:anchor`: the whole file would be included; includes_resolve reports it
            _ => found.push((spec.to_string(), String::new())),
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
        let name = chapter
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let text = fs::read_to_string(&chapter).expect("chapter is readable");
        for (path, anchor) in includes(&text) {
            let target = chapter.parent().unwrap_or(Path::new(".")).join(&path);
            let Ok(content) = fs::read_to_string(&target) else {
                problems.push(format!("{name}: {path} does not exist (anchor {anchor})"));
                continue;
            };
            if anchor.is_empty() {
                problems.push(format!(
                    "{name}: the include of {path} has no :anchor (it would pull in the whole file)"
                ));
                continue;
            }
            let lines: Vec<&str> = content.lines().collect();
            if anchor_range(&lines, &anchor).is_none() {
                problems.push(format!(
                    "{name}: {path} has no single ANCHOR/ANCHOR_END pair named {anchor}"
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "broken includes:\n{}",
        problems.join("\n")
    );
}

/// README sections that the book covers with hand-written pages instead of an include: heading, book page.
const README_COVERED_BY: &[(&str, &str)] = &[
    ("## Usage", "reference/cli.md"),
    ("## Script API index", "guides/script-api.md"),
    ("## MCP server", "guides/mcp.md"),
    (
        "## Inspecting models and checking their textures",
        "guides/textures.md",
    ),
    ("## Binarizing models on Linux", "guides/binarize-linux.md"),
];

#[test]
fn every_readme_section_is_in_the_book_or_mapped() {
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
        let heads_range = ranges
            .iter()
            .any(|&(s, _)| readme.get(s).is_some_and(|l| *l == next));
        let mapped = README_COVERED_BY
            .iter()
            .any(|(h, page)| h == line && book_src().join(page).is_file());
        if !inside && !heads_range && !mapped {
            missing.push((*line).to_string());
        }
    }
    assert!(
        missing.is_empty(),
        "README sections that no book chapter includes and README_COVERED_BY does not map to an existing page: {missing:?}"
    );
}

/// The text of every `.md` file below `docs/book/src/reference`, as `(file name, text)`.
fn reference_pages() -> Vec<(String, String)> {
    let dir = book_src().join("reference");
    let mut pages = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.map(|e| e.expect("directory entry").path()) {
            if entry.extension().is_some_and(|e| e == "md") {
                let name = entry
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                pages.push((name, fs::read_to_string(&entry).expect("page is readable")));
            }
        }
    }
    pages.sort();
    pages
}

/// The lines of `text` from the heading line `heading` to the next heading of the same or a higher level.
fn section<'a>(text: &'a str, heading: &str) -> Option<Vec<&'a str>> {
    let level = heading.chars().take_while(|c| *c == '#').count();
    let mut lines = text
        .lines()
        .skip_while(|l| l.trim_end() != heading)
        .peekable();
    lines.next()?;
    let mut body = Vec::new();
    for line in lines {
        let l = line.chars().take_while(|c| *c == '#').count();
        if l > 0 && l <= level && line.chars().nth(l) == Some(' ') {
            break;
        }
        body.push(line);
    }
    Some(body)
}

/// The section `heading` in any of the reference pages.
fn reference_section(heading: &str) -> Option<String> {
    reference_pages()
        .iter()
        .find_map(|(_, text)| section(text, heading))
        .map(|lines| lines.join("\n"))
}

/// The usage lines of the CLI as `(heading name, line)`, e.g. `("api class", "dayzmake api class ...")`.
fn usage_commands() -> Vec<(String, String)> {
    // the lines between `Usage:` and the first blank line, whatever their indentation
    let block: Vec<&str> = dayzmake::run::USAGE
        .lines()
        .skip_while(|l| l.trim() != "Usage:")
        .skip(1)
        .take_while(|l| !l.trim().is_empty())
        .map(str::trim_start)
        .collect();
    let lines = block.iter().filter(|l| l.starts_with("dayzmake ")).count();
    let mut commands = Vec::new();
    let mut global = 0;
    for line in &block {
        let Some(rest) = line.strip_prefix("dayzmake ") else {
            continue;
        };
        if rest.starts_with(['(', '-']) {
            global += 1; // `dayzmake (-h | --help)` and `dayzmake --version`
            continue;
        }
        let mut words = rest.split_whitespace();
        let first = words.next().unwrap_or("");
        let name = if first == "api" {
            format!("api {}", words.next().unwrap_or(""))
        } else {
            first.to_string()
        };
        commands.push((name, (*line).to_string()));
    }
    // a changed layout of USAGE must not make the parsing pass without checking anything
    assert_eq!(
        commands.len() + global,
        lines,
        "every `dayzmake ` line of the usage block must be a command or a global form"
    );
    assert!(
        commands.len() >= 24,
        "only {} commands found in the usage block of dayzmake::run::USAGE",
        commands.len()
    );
    commands
}

#[test]
fn every_command_has_a_reference_section() {
    let mut problems = Vec::new();
    for (name, line) in usage_commands() {
        match reference_section(&format!("## dayzmake {name}")) {
            None => problems.push(format!(
                "no heading \"## dayzmake {name}\" in docs/book/src/reference/"
            )),
            Some(body) if !body.contains(&line) => problems.push(format!(
                "the section of dayzmake {name} does not contain its usage line:\n    {line}"
            )),
            Some(_) => {}
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Short option to long option, from the `Options:` block of the usage text (`-v --verbose`).
fn long_names() -> Vec<(String, String)> {
    let mut pairs = Vec::new();
    let mut in_options = false;
    for line in dayzmake::run::USAGE.lines() {
        if line.starts_with("Options:") {
            in_options = true;
            continue;
        }
        if !in_options {
            continue;
        }
        let mut words = line.split_whitespace();
        if let (Some(a), Some(b)) = (words.next(), words.next())
            && a.starts_with('-')
            && !a.starts_with("--")
            && b.starts_with("--")
        {
            pairs.push((a.to_string(), b.to_string()));
        }
    }
    pairs
}

#[test]
fn every_option_is_documented_with_its_command() {
    let longs = long_names();
    let mut problems = Vec::new();
    for (name, line) in usage_commands() {
        let Some(body) = reference_section(&format!("## dayzmake {name}")) else {
            continue; // reported by every_command_has_a_reference_section
        };
        for token in line.split_whitespace() {
            let option = token.trim_matches(|c| c == '[' || c == ']' || c == '.');
            if !option.starts_with('-') || option.len() < 2 {
                continue;
            }
            let alt = longs
                .iter()
                .find(|(s, _)| s == option)
                .map(|(_, l)| l.clone());
            // `-w`, `-w <wname>` (an option with its argument) and the long form all count
            let named =
                |o: &str| body.contains(&format!("`{o}`")) || body.contains(&format!("`{o} "));
            let documented = named(option) || alt.is_some_and(|l| named(&l));
            if !documented {
                problems.push(format!(
                    "dayzmake {name}: option {option} is not documented"
                ));
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The tools `dayzmake mcp` lists, as `(name, property names)`.
fn mcp_tools() -> Vec<(String, Vec<String>)> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut child = Command::new(env!("CARGO_BIN_EXE_dayzmake"))
        .arg("mcp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn dayzmake mcp");
    child
        .stdin
        .take()
        .expect("stdin")
        .write_all(b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"tools/list\"}\n")
        .expect("write the request");
    let out = child.wait_with_output().expect("server output");
    let reply: serde_json::Value = serde_json::from_slice(&out.stdout).expect("one JSON-RPC reply");
    let tools: Vec<(String, Vec<String>)> = reply["result"]["tools"]
        .as_array()
        .expect("a tool list")
        .iter()
        .map(|t| {
            let props = t["inputSchema"]["properties"]
                .as_object()
                .map(|o| o.keys().cloned().collect())
                .unwrap_or_default();
            (t["name"].as_str().expect("tool name").to_string(), props)
        })
        .collect();
    assert!(!tools.is_empty(), "tools/list returned no tools");
    tools
}

#[test]
fn every_mcp_tool_is_documented() {
    let page = fs::read_to_string(book_src().join("reference/mcp.md")).unwrap_or_default();
    let mut problems = Vec::new();
    for (tool, props) in mcp_tools() {
        match section(&page, &format!("### {tool}")) {
            None => problems.push(format!("no heading \"### {tool}\" in reference/mcp.md")),
            Some(body) => {
                let body = body.join("\n");
                for prop in props {
                    if !body.contains(&format!("`{prop}`")) {
                        problems.push(format!("{tool}: argument {prop} is not documented"));
                    }
                }
            }
        }
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// Every warning name the sources raise: the string after `Some(` in `Warning::new(Some("name"), ...)`
/// and the config parser's `Some("name")` tuples, plus the two names of the model texture check, which
/// `p3d.rs` keeps in a table. There is no central list in dayzmake-core, so the sources are the list.
fn warning_names() -> Vec<String> {
    let mut names = std::collections::BTreeSet::new();
    let mut files = Vec::new();
    for dir in ["src", "crates/dayzmake-core/src"] {
        let mut stack = vec![root().join(dir)];
        while let Some(d) = stack.pop() {
            for entry in fs::read_dir(d)
                .expect("source folder")
                .map(|e| e.expect("entry").path())
            {
                if entry.is_dir() {
                    stack.push(entry);
                } else if entry.extension().is_some_and(|e| e == "rs") {
                    files.push(entry);
                }
            }
        }
    }
    for file in files {
        let text = fs::read_to_string(&file).expect("source file");
        for marker in ["Some(", "(\"texture\", ", "(\"material\", "] {
            let mut rest = text.as_str();
            while let Some(at) = rest.find(marker) {
                rest = &rest[at + marker.len()..];
                let trimmed = rest.trim_start();
                if let Some(lit) = trimmed.strip_prefix('"')
                    && let Some(end) = lit.find('"')
                {
                    let name = &lit[..end];
                    let kebab = name.contains('-')
                        && name
                            .chars()
                            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
                        && !name.starts_with('-')
                        && !name.ends_with('-');
                    if kebab {
                        names.insert(name.to_string());
                    }
                }
            }
        }
    }
    names.into_iter().collect()
}

#[test]
fn every_warning_is_documented() {
    let page = fs::read_to_string(book_src().join("reference/warnings.md")).unwrap_or_default();
    let headings: Vec<String> = page
        .lines()
        .filter(|l| l.starts_with('#'))
        .map(|l| {
            l.trim_start_matches('#')
                .trim()
                .trim_matches('`')
                .to_string()
        })
        .collect();
    let names = warning_names();
    assert!(names.len() >= 20, "the scan found only {names:?}");
    let missing: Vec<&String> = names.iter().filter(|n| !headings.contains(n)).collect();
    assert!(
        missing.is_empty(),
        "warnings without a heading in reference/warnings.md: {missing:?}"
    );
}

/// Targets of Markdown links in `lines` that are not absolute. Lines inside a fenced code block are code, not Markdown.
fn relative_link_targets(lines: &[&str]) -> Vec<String> {
    let mut bad = Vec::new();
    let mut in_fence = false;
    for line in lines {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
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
    bad
}

#[test]
fn relative_link_check_skips_fenced_code() {
    let lines = [
        "[abs](https://example.org) [rel](docs/a.md)",
        "```",
        "[bool]($s | Select-String -Quiet)",
        "```",
        "after [rel2](b.md)",
    ];
    assert_eq!(relative_link_targets(&lines), ["docs/a.md", "b.md"]);
}

#[test]
fn included_readme_links_are_absolute() {
    let text = readme_text();
    let readme: Vec<&str> = text.lines().collect();
    let mut bad = Vec::new();
    for (start, end) in included_readme_ranges(&readme) {
        bad.extend(relative_link_targets(
            readme.get(start..end).unwrap_or_default(),
        ));
    }
    assert!(
        bad.is_empty(),
        "relative links in README parts that the book includes (404 on the site): {bad:?}"
    );
}
