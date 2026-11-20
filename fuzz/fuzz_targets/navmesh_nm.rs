#![no_main]

use dayzmake_core::navmesh::detour::{NavMesh, NavMeshSet, TileData};
use dayzmake_core::navmesh::islands::filter_islands;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // The first byte selects the reader: odd = a single tile blob, even = a whole `.nm` file.
    let Some((&selector, data)) = data.split_first() else {
        return;
    };
    if selector & 1 == 1 {
        if let Ok(tile) = TileData::parse(data) {
            // Compare bytes, not values: floats may be NaN, which is never equal to itself. A blob
            // that parses is canonical (counts and sizes are derived and were checked equal), so
            // the first write reproduces the input exactly.
            let bytes = tile.to_bytes();
            assert_eq!(bytes, data, "write(parse(d)) != d");
            let again = TileData::parse(&bytes).expect("serialised tile parses");
            assert_eq!(again.to_bytes(), bytes);
        }
        return;
    }
    if let Ok(set) = NavMeshSet::read(&mut &data[..]) {
        // Linking (`from_set` recomputes every link) and the island flood see the same hostile tiles.
        if let Ok(mut mesh) = NavMesh::from_set(&set) {
            let centre = set.tiles.first().map_or([0.0; 3], |t| {
                let (lo, hi) = (t.data.header.bmin, t.data.header.bmax);
                [0, 1, 2].map(|i| lo[i] * 0.5 + hi[i] * 0.5)
            });
            for seed in [centre, [f32::NAN; 3], [0.0; 3]] {
                let first = filter_islands(&mut mesh, seed);
                // The filter clears its own flags first, so a second run gives the same result.
                assert_eq!(filter_islands(&mut mesh, seed), first);
            }
        }
        // Files that read can still be unwritable (a slot at or above maxTiles, two tiles in one
        // slot), and a file is not canonical (version 7, ignored offset table, padding of the
        // table), so only the second write is compared for whole files; the tile blobs inside
        // are compared exactly through the tile selector. What writes must read and write again
        // to the same bytes.
        let mut bytes = Vec::new();
        if set.write(&mut bytes).is_ok() {
            let again = NavMeshSet::read(&mut &bytes[..]).expect("written file reads");
            let mut second = Vec::new();
            again.write(&mut second).expect("a file that was written writes again");
            assert_eq!(second, bytes);
        }
    }
});
