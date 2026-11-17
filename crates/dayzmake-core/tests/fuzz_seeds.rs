//! Runs every fuzz corpus seed (and every committed crash regression input) through the
//! same readers the fuzz targets call, so that `cargo test` catches panics without nightly.

use std::fs;
use std::io::{self, Cursor};
use std::path::Path;

use dayzmake_core::config::Config;
#[cfg(feature = "navmesh")]
use dayzmake_core::navmesh::capture::{Capture, CaptureWriter};
#[cfg(feature = "navmesh")]
use dayzmake_core::navmesh::detour::{NavMeshSet, TileData};
#[cfg(feature = "navmesh")]
use dayzmake_core::navmesh::wire::{TileInput, WorldParams};
use dayzmake_core::odol::Odol;
use dayzmake_core::p3d::P3D;
use dayzmake_core::paa::Paa;
use dayzmake_core::pbo::PBO;
#[cfg(feature = "sign")]
use dayzmake_core::sign::{BIPrivateKey, BIPublicKey, BISign};
use dayzmake_core::warning::Warnings;

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
fn config_parse_seeds() -> io::Result<()> {
    for data in seeds("config_parse")? {
        let input = String::from_utf8_lossy(&data).into_owned();
        // Same as the fuzz target: includes (also `# include`, `#\tinclude`) read host files, so any
        // seed containing "include" is skipped.
        if input.contains("include") {
            continue;
        }
        let mut warnings = Warnings::default();
        let _ = Config::from_string(input, None, &[], &mut warnings);
    }
    Ok(())
}

#[test]
fn paa_read_seeds() -> io::Result<()> {
    for data in seeds("paa_read")? {
        if let Ok(paa) = Paa::read(&mut Cursor::new(&data)) {
            #[cfg(feature = "image")]
            let _ = paa.to_dynamic_image();
            #[cfg(not(feature = "image"))]
            let _ = paa;
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

#[cfg(feature = "sign")]
#[test]
fn sign_read_seeds() -> io::Result<()> {
    for data in seeds("sign_read")? {
        let _ = BIPrivateKey::read(&mut &data[..]);
        let _ = BIPublicKey::read(&mut &data[..]);
        let _ = BISign::read(&mut &data[..]);
    }
    Ok(())
}

#[cfg(feature = "sign")]
#[test]
fn sign_verify_seeds() -> io::Result<()> {
    use dayzmake_core::sign::BISignVersion;

    // the same layout and calls as the fuzz target `sign_verify`
    let template = fs::read(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/bi/sign/testsig.pbo"),
    )?;
    for data in seeds("sign_verify")? {
        let [a, b, c, d, rest @ ..] = &data[..] else {
            continue;
        };
        let key_len = usize::from(u16::from_be_bytes([*a, *b])).min(rest.len());
        let (key, rest) = rest.split_at(key_len);
        let sig_len = usize::from(u16::from_be_bytes([*c, *d])).min(rest.len());
        let (sig, private) = rest.split_at(sig_len);
        let pbo = PBO::read(&mut Cursor::new(&template))?;
        if let (Ok(key), Ok(sig)) = (
            BIPublicKey::read(&mut &key[..]),
            BISign::read(&mut &sig[..]),
        ) {
            let _ = key.verify(&pbo, &sig);
        }
        if let Ok(private) = BIPrivateKey::read(&mut &private[..]) {
            let _ = private.sign(&pbo, BISignVersion::V3);
        }
    }
    Ok(())
}

#[test]
fn odol_read_seeds() -> io::Result<()> {
    for data in seeds("odol_read")? {
        let _ = Odol::read(&mut Cursor::new(&data));
    }
    Ok(())
}

#[cfg(feature = "navmesh")]
#[test]
fn navmesh_wire_seeds() -> io::Result<()> {
    for seed in seeds("navmesh_wire")? {
        // Same as the fuzz target: the first byte selects the reader.
        let Some((&selector, data)) = seed.split_first() else {
            continue;
        };
        if selector & 1 == 1 {
            if let Ok(capture) = Capture::read(&mut &data[..]) {
                let mut out = Vec::new();
                let mut w = CaptureWriter::new(&mut out, capture.params())?;
                for t in capture.tiles() {
                    w.add(t)?;
                }
                let again = Capture::read(&mut &out[..])?;
                assert_eq!(again.params().encode(), capture.params().encode());
                assert_eq!(again.tiles().len(), capture.tiles().len());
                for (a, b) in again.tiles().iter().zip(capture.tiles()) {
                    assert_eq!(a.encode(), b.encode());
                }
            }
            continue;
        }
        // What parses must encode and parse again to the same bytes.
        if let Ok(world) = WorldParams::parse(data) {
            let bytes = world.encode();
            assert_eq!(WorldParams::parse(&bytes)?.encode(), bytes);
        }
        let _ = WorldParams::parse_message(data);
        if let Ok(tile) = TileInput::parse(data) {
            let bytes = tile.encode();
            let again = TileInput::parse(&bytes)?;
            assert_eq!(again.encode(), bytes);
        }
    }
    Ok(())
}

#[cfg(feature = "navmesh")]
#[test]
fn navmesh_nm_seeds() -> io::Result<()> {
    for seed in seeds("navmesh_nm")? {
        // Same as the fuzz target: the first byte selects the reader.
        let Some((&selector, data)) = seed.split_first() else {
            continue;
        };
        if selector & 1 == 1 {
            if let Ok(tile) = TileData::parse(data) {
                let bytes = tile.to_bytes();
                assert_eq!(bytes, data, "write(parse(d)) != d");
                assert_eq!(TileData::parse(&bytes)?.to_bytes(), bytes);
            }
            continue;
        }
        if let Ok(set) = NavMeshSet::read(&mut &data[..]) {
            let mut bytes = Vec::new();
            if set.write(&mut bytes).is_ok() {
                let again = NavMeshSet::read(&mut &bytes[..])?;
                let mut second = Vec::new();
                again.write(&mut second)?;
                assert_eq!(second, bytes);
            }
        }
    }
    Ok(())
}
