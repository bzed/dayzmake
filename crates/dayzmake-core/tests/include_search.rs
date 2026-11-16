//! Where an absolute include is looked for (#66): the working directory is only checked directly, `-i` folders are
//! searched recursively without following symlinks out of the folder or into a loop.
use std::fs;
use std::path::{Path, PathBuf};

use tempfile::tempdir;

use dayzmake_core::preprocess::{IncludeFolders, preprocess_in};
use dayzmake_core::warning::Warnings;

const INPUT: &str = "#include \"\\DZ\\data\\x.hpp\"\n";

#[expect(clippy::unwrap_used, reason = "test helper")]
fn put(dir: &Path, relative: &str, content: &str) -> PathBuf {
    let path = dir.join(relative);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, content).unwrap();
    path
}

fn run(folders: &IncludeFolders) -> Result<String, String> {
    preprocess_in(INPUT.to_string(), None, folders, &mut Warnings::new())
        .map(|(out, _)| out)
        .map_err(|e| e.to_string())
}

#[test]
fn direct_folder_finds_the_file_directly_below_it() {
    let cwd = tempdir().unwrap();
    put(cwd.path(), "DZ/data/x.hpp", "a = 1;\n");
    let folders = IncludeFolders::new(Vec::new()).with_direct(vec![cwd.path().to_path_buf()]);
    assert_eq!(run(&folders).unwrap().trim(), "a = 1;");
}

#[test]
fn direct_folder_is_not_walked() {
    let cwd = tempdir().unwrap();
    // a big tree with the file buried in it: only a recursive search would find it
    for i in 0..50 {
        put(cwd.path(), &format!("d{i}/f.txt"), "x");
    }
    put(cwd.path(), "deep/er/DZ/data/x.hpp", "a = 1;\n");
    let folders = IncludeFolders::new(Vec::new()).with_direct(vec![cwd.path().to_path_buf()]);
    let err = run(&folders).unwrap_err();
    assert!(err.contains("not found"), "{err}");

    // the same folder as a search folder finds it
    let folders = IncludeFolders::new(vec![cwd.path().to_path_buf()]);
    assert_eq!(run(&folders).unwrap().trim(), "a = 1;");
}

#[test]
fn direct_folder_matches_case_insensitively() {
    let cwd = tempdir().unwrap();
    put(cwd.path(), "dz/Data/X.hpp", "a = 1;\n");
    let folders = IncludeFolders::new(Vec::new()).with_direct(vec![cwd.path().to_path_buf()]);
    assert_eq!(run(&folders).unwrap().trim(), "a = 1;");
}

#[test]
fn search_folder_finds_nested_and_case_insensitive_files() {
    let inc = tempdir().unwrap();
    put(inc.path(), "a/b/dz/DATA/X.hpp", "a = 2;\n");
    let folders = IncludeFolders::new(vec![inc.path().to_path_buf()]);
    assert_eq!(run(&folders).unwrap().trim(), "a = 2;");
}

#[test]
fn search_folders_win_over_direct_ones() {
    let inc = tempdir().unwrap();
    let cwd = tempdir().unwrap();
    put(inc.path(), "m/DZ/data/x.hpp", "a = 1;\n");
    put(cwd.path(), "DZ/data/x.hpp", "a = 2;\n");
    let folders = IncludeFolders::new(vec![inc.path().to_path_buf()])
        .with_direct(vec![cwd.path().to_path_buf()]);
    assert_eq!(run(&folders).unwrap().trim(), "a = 1;");
}

#[cfg(unix)]
#[test]
fn symlink_loop_under_a_search_folder_terminates() {
    let inc = tempdir().unwrap();
    put(inc.path(), "a/f.txt", "x");
    std::os::unix::fs::symlink(inc.path(), inc.path().join("a/back")).unwrap();
    std::os::unix::fs::symlink(inc.path().join("a"), inc.path().join("a/self")).unwrap();
    let folders = IncludeFolders::new(vec![inc.path().to_path_buf()]);
    let err = run(&folders).unwrap_err();
    // the walk ended by itself, not by the entry cap
    assert!(
        err.contains("not found") && !err.contains("stopped after"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn symlink_out_of_a_search_folder_is_not_followed() {
    let inc = tempdir().unwrap();
    let outside = tempdir().unwrap();
    put(outside.path(), "DZ/data/x.hpp", "a = 3;\n");
    std::os::unix::fs::symlink(outside.path(), inc.path().join("link")).unwrap();
    let folders = IncludeFolders::new(vec![inc.path().to_path_buf()]);
    let err = run(&folders).unwrap_err();
    assert!(err.contains("not found"), "{err}");
}

#[cfg(unix)]
#[expect(clippy::unwrap_used, reason = "test helper")]
fn link_maze(inc: &Path) {
    use std::os::unix::fs::symlink;
    fs::create_dir_all(inc.join("a")).unwrap();
    fs::create_dir_all(inc.join("b")).unwrap();
    symlink("../b", inc.join("a/x")).unwrap();
    symlink(".", inc.join("b/y")).unwrap();
    symlink("../b", inc.join("b/z")).unwrap();
}

/// Loops through several links branch exponentially unless a directory is entered only once.
#[cfg(unix)]
#[test]
fn link_maze_fails_fast_with_not_found() {
    let inc = tempdir().unwrap();
    link_maze(inc.path());
    let folders = IncludeFolders::new(vec![inc.path().to_path_buf()]);
    let err = run(&folders).unwrap_err();
    assert!(
        err.contains("not found") && !err.contains("stopped after"),
        "{err}"
    );
}

#[cfg(unix)]
#[test]
fn link_maze_still_finds_the_file_elsewhere() {
    let inc = tempdir().unwrap();
    link_maze(inc.path());
    put(inc.path(), "zz/DZ/data/x.hpp", "a = 4;\n");
    let folders = IncludeFolders::new(vec![inc.path().to_path_buf()]);
    assert_eq!(run(&folders).unwrap().trim(), "a = 4;");
}
