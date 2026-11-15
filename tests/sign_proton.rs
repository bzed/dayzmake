//! Needs DayZ Tools and Proton: run with `DAYZMAKE_PROTON_TESTS=1 DISPLAY=:0 cargo test --test sign_proton -- --ignored`.
//!
//! A key from `dayzmake keygen` signs a PBO with BI's `DSSignFile.exe`; `DSCheckSignatures.exe -deep` accepts that
//! signature and one made by `dayzmake sign`; `dayzmake verify` accepts the BI signature; and both signatures are
//! byte-identical. `STEAM_ROOT`, `DAYZ_TOOLS` (the DayZ Tools folder) and `PROTON` (the Proton directory) override
//! the discovery of Steam, DayZ Tools and Proton Hotfix; the test skips when one is missing. `DsUtils` is copied into a
//! sandbox under /var/tmp and run from there, so the install is only read.
use std::path::{Path, PathBuf};
use std::process::Command;

fn steam_root() -> Option<PathBuf> {
    if let Ok(s) = std::env::var("STEAM_ROOT") {
        return Some(PathBuf::from(s));
    }
    let home = PathBuf::from(std::env::var_os("HOME")?);
    [".local/share/Steam", ".steam/steam", ".steam/root"]
        .iter()
        .map(|p| home.join(p))
        .find(|p| p.join("steamapps/common").is_dir())
}

fn find_tools_and_proton() -> Option<(PathBuf, PathBuf, PathBuf)> {
    let steam = steam_root()?;
    let common = steam.join("steamapps/common");
    let tools = std::env::var("DAYZ_TOOLS")
        .map_or_else(|_| common.join("DayZ Tools"), PathBuf::from)
        .join("Bin/DsUtils");
    let proton =
        std::env::var("PROTON").map_or_else(|_| common.join("Proton Hotfix"), PathBuf::from);
    (tools.join("DSSignFile.exe").is_file() && proton.join("proton").is_file())
        .then_some((steam, tools, proton))
}

#[expect(clippy::unwrap_used, reason = "test helper")]
fn dayzmake(args: &[&str], cwd: &Path, home: &Path) -> (i32, String) {
    let out = isolated(Command::new(env!("CARGO_BIN_EXE_dayzmake")), home)
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (
        out.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// Points the `XDG_*` variables of `cmd` into `home`, so nothing is written to the user's folders.
fn xdg_isolated(mut cmd: Command, home: &Path) -> Command {
    cmd.env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("XDG_CACHE_HOME", home.join(".cache"))
        .env("XDG_STATE_HOME", home.join(".local/state"));
    cmd
}

/// Like `xdg_isolated`, and `HOME` too. Not for Proton: it hangs without the real `HOME`.
fn isolated(mut cmd: Command, home: &Path) -> Command {
    cmd.env("HOME", home);
    xdg_isolated(cmd, home)
}

struct Wine {
    steam: PathBuf,
    proton: PathBuf,
    sandbox: PathBuf,
}

impl Drop for Wine {
    /// Waits for the prefix's wineserver to exit, so none keeps the sandbox directory alive.
    fn drop(&mut self) {
        // best effort: a failure only leaves a wineserver that exits by itself a few seconds later
        let _ = xdg_isolated(
            Command::new(self.proton.join("files/bin/wineserver")),
            &self.sandbox,
        )
        .arg("-w")
        .env("WINEPREFIX", self.sandbox.join("pfx/pfx"))
        .status();
    }
}

impl Wine {
    #[expect(clippy::unwrap_used, reason = "test helper")]
    fn proton(&self, verb: &str, args: &[&str]) -> (bool, String) {
        // Output goes to a file, not a pipe: wineserver outlives the command and would keep a pipe open.
        let log = self.sandbox.join("proton.log");
        let file = std::fs::File::create(&log).unwrap();
        let status = xdg_isolated(Command::new(self.proton.join("proton")), &self.sandbox)
            .arg(verb)
            .args(args)
            .current_dir(&self.sandbox)
            .env("STEAM_COMPAT_CLIENT_INSTALL_PATH", &self.steam)
            .env("STEAM_COMPAT_DATA_PATH", self.sandbox.join("pfx"))
            .env("SteamAppId", "830640")
            .env("SteamGameId", "830640")
            .env("STEAM_COMPAT_APP_ID", "830640")
            .stdin(std::process::Stdio::null())
            .stdout(file.try_clone().unwrap())
            .stderr(file)
            .status()
            .unwrap();
        (
            status.success(),
            std::fs::read_to_string(&log).unwrap_or_default(),
        )
    }

    /// Runs `command` through `cmd /c` with its output redirected into `out/<name>` (the DS tools print nothing
    /// otherwise), one command at a time, and returns that output.
    #[expect(clippy::panic, reason = "test helper")]
    fn tool(&self, name: &str, command: &str) -> String {
        let line = format!("{command} > Z:\\out\\{name} 2>&1");
        let (ok, err) = self.proton("waitforexitandrun", &["cmd", "/c", &line]);
        assert!(ok, "{command}: {err}");
        std::fs::read_to_string(self.sandbox.join("out").join(name))
            .unwrap_or_else(|e| panic!("{command}: no output file: {e}"))
    }
}

#[test]
#[ignore = "needs DayZ Tools"]
#[expect(clippy::print_stderr, reason = "skip notice")]
fn dayz_tools_and_dayzmake_accept_each_others_keys_and_signatures() {
    if std::env::var("DAYZMAKE_PROTON_TESTS").as_deref() != Ok("1") {
        eprintln!("skipped: set DAYZMAKE_PROTON_TESTS=1");
        return;
    }
    if std::env::var_os("DISPLAY").is_none() && std::env::var_os("WAYLAND_DISPLAY").is_none() {
        // Wine hangs while creating a prefix without a display
        eprintln!("skipped: Wine needs a display (DISPLAY)");
        return;
    }
    let Some((steam, tools, proton)) = find_tools_and_proton() else {
        eprintln!("skipped: DayZ Tools or Proton not found (STEAM_ROOT, DAYZ_TOOLS, PROTON)");
        return;
    };
    let fixtures = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/bi/sign");
    std::fs::create_dir_all("/var/tmp").unwrap();
    let tmp = tempfile::Builder::new()
        .prefix("dayzmake-sign-proton.")
        .tempdir_in("/var/tmp")
        .unwrap();
    let root = tmp.path().to_path_buf();
    for dir in ["pfx", "DsUtils", "in", "out", "bi", "mine", "keys"] {
        std::fs::create_dir_all(root.join(dir)).unwrap();
    }
    for entry in std::fs::read_dir(&tools).unwrap() {
        let path = entry.unwrap().path();
        std::fs::copy(&path, root.join("DsUtils").join(path.file_name().unwrap())).unwrap();
    }
    for dir in ["in", "bi", "mine"] {
        std::fs::copy(fixtures.join("testsig.pbo"), root.join(dir).join("a.pbo")).unwrap();
    }
    let wine = Wine {
        steam,
        proton,
        sandbox: root.clone(),
    };

    // create the prefix, then make Z: the sandbox
    let (ok, err) = wine.proton("waitforexitandrun", &["cmd", "/c", "ver"]);
    assert!(ok, "creating the prefix: {err}");
    let z = root.join("pfx/pfx/dosdevices/z:");
    let _ = std::fs::remove_file(&z); // the prefix's own z: link (to /) may or may not exist
    #[cfg(unix)]
    std::os::unix::fs::symlink(&root, &z).unwrap();

    let (code, err) = dayzmake(&["keygen", "gen"], &root.join("in"), &root);
    assert_eq!(code, 0, "{err}");
    std::fs::copy(
        root.join("in/gen.biprivatekey"),
        root.join("DsUtils/gen.biprivatekey"),
    )
    .unwrap();
    std::fs::copy(root.join("in/gen.bikey"), root.join("keys/gen.bikey")).unwrap();
    std::fs::copy(
        root.join("in/gen.biprivatekey"),
        root.join("mine/gen.biprivatekey"),
    )
    .unwrap();

    // BI signs: DSSignFile writes a.pbo.gen.bisign next to the PBO
    let out = wine.tool(
        "sign.txt",
        r"Z:\DsUtils\DSSignFile.exe Z:\DsUtils\gen.biprivatekey Z:\in\a.pbo",
    );
    eprintln!("DSSignFile: {out}");
    let bi_sig = root.join("in/a.pbo.gen.bisign");
    assert!(bi_sig.is_file(), "DSSignFile wrote no signature: {out}");
    std::fs::copy(&bi_sig, root.join("bi/a.pbo.gen.bisign")).unwrap();

    // dayzmake signs the same PBO with the same key
    let (code, err) = dayzmake(
        &["sign", "gen.biprivatekey", "a.pbo"],
        &root.join("mine"),
        &root,
    );
    assert_eq!(code, 0, "{err}");
    let mine_sig = root.join("mine/a.pbo.gen.bisign");
    assert_eq!(
        std::fs::read(&bi_sig).unwrap(),
        std::fs::read(&mine_sig).unwrap(),
        "BI and dayzmake signatures differ"
    );

    // dayzmake verifies the BI signature
    let (code, err) = dayzmake(
        &["verify", "../keys/gen.bikey", "a.pbo", "a.pbo.gen.bisign"],
        &root.join("bi"),
        &root,
    );
    assert_eq!(code, 0, "dayzmake verify of the BI signature: {err}");

    // DSCheckSignatures accepts both
    for (dir, name) in [("bi", "check-bi.txt"), ("mine", "check-mine.txt")] {
        let command = format!(r"Z:\DsUtils\DSCheckSignatures.exe -deep Z:\{dir} Z:\keys");
        let out = wine.tool(name, &command);
        eprintln!("DSCheckSignatures {dir}: {out}");
        assert!(
            out.contains("is OK"),
            "DSCheckSignatures rejected the {dir} signature: {out}"
        );
    }
}
