//! Test helper: PBO bytes built by hand, so a test can hold entry names and header extensions that
//! `PBO::insert_entry` refuses (`..\evil.txt`, absolute paths, ...). The trailer is 20 zero bytes;
//! `PBO::read` does not check it.

/// A PBO holding exactly `entries` (name, stored bytes) in the given order, with `extensions`
/// (key, value) in the header.
#[expect(clippy::expect_used, reason = "test helper")]
pub fn raw_pbo(entries: &[(&str, &[u8])], extensions: &[(&str, &str)]) -> Vec<u8> {
    let entries: Vec<(&str, &[u8], u32, u32)> = entries
        .iter()
        .map(|(name, data)| (*name, *data, 0, u32::try_from(data.len()).expect("small")))
        .collect();
    raw_pbo_with_headers(&entries, extensions)
}

/// Packing method of LZSS-compressed ("Cprs") entries.
#[allow(dead_code, reason = "not every test crate uses it")]
pub const CPRS: u32 = 0x4370_7273;

/// A literal-only LZSS stream the reader decodes to `data`: flag byte `0xFF` (eight literals),
/// then those literals, repeated. No trailing checksum (the reader ignores it).
#[allow(dead_code, reason = "not every test crate uses it")]
pub fn lzss_literals(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    for group in data.chunks(8) {
        out.push(0xFF);
        out.extend_from_slice(group);
    }
    out
}

/// Like [`raw_pbo`], with a packing method and original size per entry:
/// `(name, stored bytes, packing method, original size)`.
#[expect(clippy::expect_used, reason = "test helper")]
pub fn raw_pbo_with_headers(
    entries: &[(&str, &[u8], u32, u32)],
    extensions: &[(&str, &str)],
) -> Vec<u8> {
    fn header(out: &mut Vec<u8>, name: &str, method: u32, original: u32, size: u32) {
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        for value in [method, original, 0, 0, size] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut out = Vec::new();
    header(&mut out, "", 0x5665_7273, 0, 0);
    for (key, value) in extensions {
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }
    out.push(0);
    for (name, data, method, original) in entries {
        let size = u32::try_from(data.len()).expect("small test entry");
        header(&mut out, name, *method, *original, size);
    }
    header(&mut out, "", 0, 0, 0);
    for (_, data, _, _) in entries {
        out.extend_from_slice(data);
    }
    out.push(0);
    out.extend_from_slice(&[0; 20]);
    out
}
