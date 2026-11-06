//! `img2paa`: the mipmaps it writes must be readable by the game.
use std::path::Path;
use std::process::Command;

/// Runs armake2 in `cwd`; returns (exit code, stderr).
fn run(args: &[&str], cwd: &Path) -> (i32, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_armake2"))
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap();
    (out.status.code().unwrap_or(-1), String::from_utf8_lossy(&out.stderr).into_owned())
}

/// A PNG of `size` x `size` pixels; `noisy` gives a picture that LZO cannot make smaller.
fn png(dir: &Path, name: &str, size: u32, noisy: bool) -> std::path::PathBuf {
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let img = image::RgbImage::from_fn(size, size, |x, y| {
        if noisy {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            image::Rgb([state as u8, (state >> 8) as u8, (state >> 16) as u8])
        } else {
            image::Rgb([(x / 4) as u8, (y / 4) as u8, 90])
        }
    });
    let path = dir.join(name);
    img.save(&path).unwrap();
    path
}

/// The mipmaps of a PAA without palette: (width with the LZO bit, height, body).
fn mipmaps(paa: &[u8]) -> Vec<(u16, u16, Vec<u8>)> {
    let mut pos = 2;
    while &paa[pos..pos + 4] == b"GGAT" {
        let len = u32::from_le_bytes(paa[pos + 8..pos + 12].try_into().unwrap()) as usize;
        pos += 12 + len;
    }
    let palette = u16::from_le_bytes([paa[pos], paa[pos + 1]]) as usize;
    pos += 2 + palette * 3;
    let mut out = Vec::new();
    loop {
        let w = u16::from_le_bytes([paa[pos], paa[pos + 1]]);
        let h = u16::from_le_bytes([paa[pos + 2], paa[pos + 3]]);
        if w == 0 && h == 0 {
            return out;
        }
        let size = paa[pos + 4] as usize | (paa[pos + 5] as usize) << 8 | (paa[pos + 6] as usize) << 16;
        out.push((w, h, paa[pos + 7..pos + 7 + size].to_vec()));
        pos += 7 + size;
    }
}

fn convert(name: &str, noisy: bool) -> (tempfile::TempDir, Vec<u8>) {
    let dir = tempfile::tempdir().unwrap();
    let source = png(dir.path(), "in.png", 512, noisy);
    let target = dir.path().join(name);
    let (code, err) = run(
        &["img2paa", "-f", "-t", "dxt1", source.to_str().unwrap(), target.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
    let bytes = std::fs::read(&target).unwrap();
    (dir, bytes)
}

#[test]
fn a_mipmap_that_lzo_cannot_shrink_is_not_marked_as_lzo() {
    let (dir, paa) = convert("noisy.paa", true);
    for (w, h, body) in mipmaps(&paa) {
        let raw = (w & 0x7FFF) as usize * h as usize / 2;
        if w & 0x8000 != 0 {
            assert!(body.len() < raw, "{w:#x} x {h}: marked as LZO but {} of {raw} bytes", body.len());
        } else {
            assert_eq!(body.len(), raw, "{w:#x} x {h}: unmarked mipmaps are raw");
        }
    }
    // the file must read back
    let png = dir.path().join("back.png");
    let (code, err) = run(
        &["paa2img", "-f", dir.path().join("noisy.paa").to_str().unwrap(), png.to_str().unwrap()],
        dir.path(),
    );
    assert_eq!(code, 0, "{err}");
}

#[test]
fn an_lzo_stream_ends_with_its_marker_and_nothing_after_it() {
    let (_dir, paa) = convert("smooth.paa", false);
    let marked: Vec<_> = mipmaps(&paa).into_iter().filter(|(w, _, _)| w & 0x8000 != 0).collect();
    assert!(!marked.is_empty(), "a smooth 512 x 512 picture compresses");
    for (w, _, body) in marked {
        assert!(body.ends_with(&[0x11, 0, 0]), "{w:#x}: no end marker");
        assert!(!body.ends_with(&[0x11, 0, 0, 0]), "{w:#x}: a byte after the end marker");
    }
}
