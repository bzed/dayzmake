#![no_main]

use dayzmake_core::navmesh::capture::{Capture, CaptureWriter};
use dayzmake_core::navmesh::wire::{TileInput, WorldParams};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte selects the reader: odd = capture file, even = the wire readers.
    let Some((&selector, data)) = data.split_first() else {
        return;
    };
    if selector & 1 == 1 {
        // What reads must write and read again to the same tiles (bytes, not values: bounds and
        // unchecked fields may hold NaN, which is never equal to itself).
        if let Ok(capture) = Capture::read(&mut &data[..]) {
            let mut out = Vec::new();
            let mut w = CaptureWriter::new(&mut out, capture.params()).expect("write to a Vec");
            for t in capture.tiles() {
                w.add(t).expect("write to a Vec");
            }
            let again = Capture::read(&mut &out[..]).expect("written capture reads");
            assert_eq!(again.params().encode(), capture.params().encode());
            assert_eq!(again.tiles().len(), capture.tiles().len());
            for (a, b) in again.tiles().iter().zip(capture.tiles()) {
                assert_eq!(a.encode(), b.encode());
            }
        }
        return;
    }
    // Both readers get every input. What parses must encode and parse again to the same bytes.
    if let Ok(world) = WorldParams::parse(data) {
        let bytes = world.encode();
        let again = WorldParams::parse(&bytes).expect("encoded world params parse");
        assert_eq!(again.encode(), bytes);
    }
    let _ = WorldParams::parse_message(data);
    if let Ok(tile) = TileInput::parse(data) {
        let bytes = tile.encode();
        let again = TileInput::parse(&bytes).expect("encoded tile parses");
        assert_eq!(again.encode(), bytes);
    }
});
