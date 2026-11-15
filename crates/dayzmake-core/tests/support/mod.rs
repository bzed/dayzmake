//! Test helper: PBO bytes built by hand, so a test can hold entry names and header extensions that
//! `PBO::insert_entry` refuses (`..\evil.txt`, absolute paths, ...). The trailer is 20 zero bytes;
//! `PBO::read` does not check it.

/// A PBO holding exactly `entries` (name, stored bytes) in the given order, with `extensions`
/// (key, value) in the header.
#[expect(clippy::expect_used, reason = "test helper")]
pub fn raw_pbo(entries: &[(&str, &[u8])], extensions: &[(&str, &str)]) -> Vec<u8> {
    fn header(out: &mut Vec<u8>, name: &str, method: u32, size: u32) {
        out.extend_from_slice(name.as_bytes());
        out.push(0);
        for value in [method, size, 0, 0, size] {
            out.extend_from_slice(&value.to_le_bytes());
        }
    }
    let mut out = Vec::new();
    header(&mut out, "", 0x5665_7273, 0);
    for (key, value) in extensions {
        out.extend_from_slice(key.as_bytes());
        out.push(0);
        out.extend_from_slice(value.as_bytes());
        out.push(0);
    }
    out.push(0);
    for (name, data) in entries {
        let size = u32::try_from(data.len()).expect("small test entry");
        header(&mut out, name, 0, size);
    }
    header(&mut out, "", 0, 0);
    for (_, data) in entries {
        out.extend_from_slice(data);
    }
    out.push(0);
    out.extend_from_slice(&[0; 20]);
    out
}
