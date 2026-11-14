//! Find the DayZ install and its workshop folder in the local Steam libraries.
//!
//! Port of `find-dayzserver.sh`. The `.vdf` and `.acf` files are untrusted: they are read with a
//! size cap and lossy UTF-8, and an `installdir` must be a safe relative path.

use std::fs::File;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest `.vdf`/`.acf` file that is read.
const MAX_VDF_BYTES: u64 = 1024 * 1024;

/// A DayZ branch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Branch {
    /// The stable branch.
    Stable,
    /// The experimental branch.
    Experimental,
}

impl Branch {
    /// Steam app id of the DayZ client.
    #[must_use]
    pub fn client_app(self) -> u32 {
        match self {
            Branch::Stable => 221_100,
            Branch::Experimental => 1_024_020,
        }
    }

    /// Steam app id of the DayZ server.
    #[must_use]
    pub fn server_app(self) -> u32 {
        match self {
            Branch::Stable => 223_350,
            Branch::Experimental => 1_042_420,
        }
    }

    /// Branch name as used on the command line and in cache names.
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Branch::Stable => "stable",
            Branch::Experimental => "experimental",
        }
    }
}

/// An installed Steam app.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppInstall {
    /// The install directory under `steamapps/common`.
    pub dir: PathBuf,
    /// The build id from the app manifest, if present.
    pub buildid: Option<String>,
}

/// Steam roots of this machine: `$STEAM_ROOT` first, then the native, Debian/Ubuntu, Flatpak and
/// Snap locations and macOS; on Windows also the registry's Steam path and `%ProgramFiles(x86)%\Steam`,
/// `%ProgramFiles%\Steam`. Only existing roots with a `steamapps` directory, deduplicated.
#[must_use]
pub fn steam_roots() -> Vec<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(root) = std::env::var_os("STEAM_ROOT").filter(|r| !r.is_empty()) {
        candidates.push(PathBuf::from(root));
    }
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        for rel in [
            ".steam/steam",
            ".steam/root",
            ".steam/debian-installation",
            ".local/share/Steam",
            ".var/app/com.valvesoftware.Steam/.local/share/Steam",
            "snap/steam/common/.local/share/Steam",
            "Library/Application Support/Steam",
        ] {
            candidates.push(home.join(rel));
        }
    }
    #[cfg(windows)]
    candidates.extend(windows_steam_roots());
    let mut roots: Vec<PathBuf> = Vec::new();
    for c in candidates {
        if !c.join("steamapps").is_dir() {
            continue;
        }
        let c = c.canonicalize().unwrap_or(c);
        if !roots.contains(&c) {
            roots.push(c);
        }
    }
    roots
}

/// Steam locations on Windows: the registry's `SteamPath`, then the default install folders. This code is
/// only compiled on Windows.
#[cfg(windows)]
fn windows_steam_roots() -> Vec<PathBuf> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;

    let mut out: Vec<PathBuf> = Vec::new();
    if let Ok(key) = RegKey::predef(HKEY_CURRENT_USER).open_subkey("Software\\Valve\\Steam")
        && let Ok(path) = key.get_value::<String, _>("SteamPath")
    {
        out.push(PathBuf::from(path));
    }
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        if let Some(dir) = std::env::var_os(var).filter(|v| !v.is_empty()) {
            out.push(PathBuf::from(dir).join("Steam"));
        }
    }
    out
}

/// Read an untrusted text file: at most 1 MiB, invalid UTF-8 replaced.
fn read_capped(path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    File::open(path)
        .ok()?
        .take(MAX_VDF_BYTES)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// The raw values of every `"key" "value"` line (see [`vdf_values_unescaped`] for paths).
#[must_use]
pub fn vdf_values<'a>(text: &'a str, key: &str) -> Vec<&'a str> {
    text.lines()
        .filter_map(|line| {
            let rest = line
                .trim()
                .strip_prefix('"')?
                .strip_prefix(key)?
                .strip_prefix('"')?;
            let rest = rest.trim_start().strip_prefix('"')?.strip_suffix('"')?;
            Some(rest)
        })
        .collect()
}

/// Like [`vdf_values`] but owned, with the `\\` escapes of Windows paths turned into `\`.
#[must_use]
pub fn vdf_values_unescaped(text: &str, key: &str) -> Vec<String> {
    vdf_values(text, key)
        .into_iter()
        .map(|v| v.replace("\\\\", "\\"))
        .collect()
}

/// Each root plus every `"path"` of its `steamapps/libraryfolders.vdf` that is an existing
/// absolute directory, deduplicated.
#[must_use]
pub fn libraries(roots: &[PathBuf]) -> Vec<PathBuf> {
    let mut libs: Vec<PathBuf> = Vec::new();
    let mut add = |p: PathBuf| {
        if !libs.contains(&p) {
            libs.push(p);
        }
    };
    for root in roots {
        add(root.clone());
        let Some(text) = read_capped(&root.join("steamapps/libraryfolders.vdf")) else {
            continue;
        };
        for value in vdf_values_unescaped(&text, "path") {
            let p = PathBuf::from(value);
            if p.is_absolute() && p.is_dir() {
                add(p);
            }
        }
    }
    libs
}

/// The first library with a manifest for `appid` whose install directory exists.
#[must_use]
pub fn find_app(libraries: &[PathBuf], appid: u32) -> Option<AppInstall> {
    libraries.iter().find_map(|lib| {
        let steamapps = lib.join("steamapps");
        let text = read_capped(&steamapps.join(format!("appmanifest_{appid}.acf")))?;
        let installdir = vdf_values_unescaped(&text, "installdir")
            .into_iter()
            .next()?;
        let rel = crate::pbo::safe_relative_path(&installdir)?;
        let dir = steamapps.join("common").join(rel);
        if !dir.is_dir() {
            return None;
        }
        let buildid = vdf_values(&text, "buildid")
            .first()
            .map(|s| (*s).to_string());
        Some(AppInstall { dir, buildid })
    })
}

/// The install of the branch that has `dta/scripts.pbo`: the client first, then the server.
#[must_use]
pub fn find_game(branch: Branch, libraries: &[PathBuf]) -> Option<AppInstall> {
    [branch.client_app(), branch.server_app()]
        .into_iter()
        .filter_map(|app| find_app(libraries, app))
        .find(|install| install.dir.join("dta/scripts.pbo").is_file())
}

/// The workshop content folder of the branch's client app, if it exists.
#[must_use]
pub fn workshop_dir(branch: Branch, libraries: &[PathBuf]) -> Option<PathBuf> {
    libraries.iter().find_map(|lib| {
        let app = branch.client_app();
        let steamapps = lib.join("steamapps");
        if !steamapps.join(format!("appmanifest_{app}.acf")).is_file() {
            return None;
        }
        let dir = steamapps.join("workshop/content").join(app.to_string());
        dir.is_dir().then_some(dir)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn manifest(root: &Path, app: u32, installdir: &str, buildid: &str) {
        let steamapps = root.join("steamapps");
        fs::create_dir_all(&steamapps).unwrap();
        fs::write(
            steamapps.join(format!("appmanifest_{app}.acf")),
            format!(
                "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app}\"\n\t\"installdir\"\t\t\"{installdir}\"\n\t\"buildid\"\t\t\"{buildid}\"\n}}\n"
            ),
        )
        .unwrap();
    }

    fn install(root: &Path, dir: &str, with_pbo: bool) {
        let d = root.join("steamapps/common").join(dir);
        fs::create_dir_all(d.join("dta")).unwrap();
        if with_pbo {
            fs::write(d.join("dta/scripts.pbo"), b"x").unwrap();
        }
    }

    #[test]
    fn vdf_paths_are_unescaped() {
        let text = "\"libraryfolders\"\n{\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\\\\x\"\n\t}\n}\n";
        assert_eq!(vdf_values(text, "path"), ["D:\\\\SteamLibrary\\\\x"]);
        assert_eq!(vdf_values_unescaped(text, "path"), ["D:\\SteamLibrary\\x"]);
    }

    #[test]
    fn finds_stable_client() {
        let t = tempfile::tempdir().unwrap();
        manifest(t.path(), 221_100, "DayZ", "1");
        install(t.path(), "DayZ", true);
        let g = find_game(Branch::Stable, &[t.path().to_path_buf()]).unwrap();
        assert_eq!(g.dir, t.path().join("steamapps/common/DayZ"));
    }

    #[test]
    fn finds_experimental_client_not_stable() {
        let t = tempfile::tempdir().unwrap();
        manifest(t.path(), 221_100, "DayZ", "1");
        install(t.path(), "DayZ", true);
        manifest(t.path(), 1_024_020, "DayZ Exp", "2");
        install(t.path(), "DayZ Exp", true);
        let libs = [t.path().to_path_buf()];
        let s = find_game(Branch::Stable, &libs).unwrap();
        let e = find_game(Branch::Experimental, &libs).unwrap();
        assert!(s.dir.ends_with("DayZ"));
        assert!(e.dir.ends_with("DayZ Exp"));
    }

    #[test]
    fn falls_back_to_server() {
        let t = tempfile::tempdir().unwrap();
        manifest(t.path(), 221_100, "DayZ", "1");
        install(t.path(), "DayZ", false);
        manifest(t.path(), 223_350, "DayZServer", "3");
        install(t.path(), "DayZServer", true);
        let g = find_game(Branch::Stable, &[t.path().to_path_buf()]).unwrap();
        assert!(g.dir.ends_with("DayZServer"));
    }

    #[test]
    fn reads_library_folders() {
        let t = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        manifest(other.path(), 221_100, "DayZ", "1");
        install(other.path(), "DayZ", true);
        fs::create_dir_all(t.path().join("steamapps")).unwrap();
        fs::write(
            t.path().join("steamapps/libraryfolders.vdf"),
            format!(
                "\"libraryfolders\"\n{{\n\t\"0\"\n\t{{\n\t\t\"path\"\t\t\"{}\"\n\t}}\n\t\"1\"\n\t{{\n\t\t\"path\"\t\t\"relative/dir\"\n\t}}\n}}\n",
                other.path().display()
            ),
        )
        .unwrap();
        let libs = libraries(&[t.path().to_path_buf()]);
        assert_eq!(
            libs,
            vec![t.path().to_path_buf(), other.path().to_path_buf()]
        );
        let g = find_game(Branch::Stable, &libs).unwrap();
        assert!(g.dir.starts_with(other.path()));
    }

    #[test]
    fn buildid_is_read() {
        let t = tempfile::tempdir().unwrap();
        manifest(t.path(), 221_100, "DayZ", "123456");
        install(t.path(), "DayZ", true);
        let a = find_app(&[t.path().to_path_buf()], 221_100).unwrap();
        assert_eq!(a.buildid.as_deref(), Some("123456"));
    }

    #[test]
    fn missing_install_is_none() {
        let t = tempfile::tempdir().unwrap();
        assert!(find_game(Branch::Stable, &[t.path().to_path_buf()]).is_none());
        // Manifest without an install directory.
        manifest(t.path(), 221_100, "DayZ", "1");
        assert!(find_app(&[t.path().to_path_buf()], 221_100).is_none());
        assert!(workshop_dir(Branch::Stable, &[t.path().to_path_buf()]).is_none());
    }

    #[test]
    fn workshop_dir_per_branch() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("steamapps/workshop/content/1024020")).unwrap();
        manifest(t.path(), 1_024_020, "DayZ Exp", "2");
        let libs = [t.path().to_path_buf()];
        assert!(workshop_dir(Branch::Stable, &libs).is_none());
        assert!(workshop_dir(Branch::Experimental, &libs).is_some());
    }

    #[test]
    fn workshop_dir_needs_client_manifest() {
        let t = tempfile::tempdir().unwrap();
        fs::create_dir_all(t.path().join("steamapps/workshop/content/221100")).unwrap();
        assert!(workshop_dir(Branch::Stable, &[t.path().to_path_buf()]).is_none());
    }

    #[test]
    fn hostile_installdir_is_skipped() {
        let t = tempfile::tempdir().unwrap();
        manifest(t.path(), 221_100, "../../..", "1");
        assert!(find_app(&[t.path().to_path_buf()], 221_100).is_none());
        manifest(t.path(), 221_100, "C:\\\\Windows", "1");
        assert!(find_app(&[t.path().to_path_buf()], 221_100).is_none());
    }

    #[test]
    fn vdf_values_parses_and_unescapes() {
        let text = "  \"path\"\t\t\"C:\\\\Games\\\\Steam\"\n\"other\" \"x\"\n";
        assert_eq!(vdf_values(text, "path"), vec!["C:\\\\Games\\\\Steam"]);
        assert_eq!(vdf_values_unescaped(text, "path"), vec!["C:\\Games\\Steam"]);
    }

    #[test]
    fn vdf_ignores_malformed_lines() {
        let text =
            "\"path\" \"unterminated\n\"path\n\"\n\"path\"\n\"path\" \"\n\u{0}\u{1}\u{ff}garbage\n";
        assert_eq!(vdf_values(text, "path").len(), 0);
        let bytes = [0xff_u8, 0xfe, b'"', 0, b'"', b'"'];
        let lossy = String::from_utf8_lossy(&bytes);
        assert_eq!(vdf_values(&lossy, "path").len(), 0);
    }
}
