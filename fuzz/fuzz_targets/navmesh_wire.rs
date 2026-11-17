#![no_main]

use dayzmake_core::navmesh::wire::{TileInput, WorldParams};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Both readers get every input. What parses must encode and parse again to the same bytes
    // (bytes, not values: bounds and unchecked fields may hold NaN, which is never equal to itself).
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
