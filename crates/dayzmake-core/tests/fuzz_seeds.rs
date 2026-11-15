//! Runs every fuzz corpus seed (and every committed crash regression input) through the
//! same readers the fuzz targets call, so that `cargo test` catches panics without nightly.

use std::fs;
use std::io::{self, Cursor};
use std::path::Path;

use dayzmake_core::config::Config;
use dayzmake_core::p3d::P3D;
use dayzmake_core::paa::Paa;
use dayzmake_core::pbo::PBO;
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign};

fn seeds(target: &str) -> io::Result<Vec<Vec<u8>>> {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../fuzz/corpus")
        .join(target);
    let mut paths = Vec::new();
    for entry in fs::read_dir(&dir)? {
        paths.push(entry?.path());
    }
    paths.sort();
    assert!(!paths.is_empty(), "no seeds in {}", dir.display());
    paths.iter().map(fs::read).collect()
}

#[test]
fn pbo_read_seeds() -> io::Result<()> {
    for data in seeds("pbo_read")? {
        let mut headers = &data[..];
        let _ = PBO::read_headers(&mut headers);
        let mut input = &data[..];
        if let Ok(pbo) = PBO::read(&mut input) {
            for name in pbo.entry_names() {
                let _ = pbo.file_data(name);
            }
        }
    }
    Ok(())
}

#[test]
fn config_rapified_seeds() -> io::Result<()> {
    for data in seeds("config_rapified")? {
        let _ = Config::read_rapified(&mut Cursor::new(&data));
    }
    Ok(())
}

#[test]
fn paa_read_seeds() -> io::Result<()> {
    for data in seeds("paa_read")? {
        if let Ok(paa) = Paa::read(&mut Cursor::new(&data)) {
            let _ = paa.to_dynamic_image();
        }
    }
    Ok(())
}

#[test]
fn p3d_read_seeds() -> io::Result<()> {
    for data in seeds("p3d_read")? {
        let mut input = &data[..];
        let _ = P3D::read(&mut input);
    }
    Ok(())
}

#[test]
fn sign_read_seeds() -> io::Result<()> {
    for data in seeds("sign_read")? {
        let _ = BIPrivateKey::read(&mut &data[..]);
        let _ = BIPublicKey::read(&mut &data[..]);
        let _ = BISign::read(&mut &data[..]);
    }
    Ok(())
}
