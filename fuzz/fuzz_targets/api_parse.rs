#![no_main]

use dayzmake::api::model::ApiIndex;
use dayzmake::api::parse::{ScriptFile, active_defines, collect_defines, index_scripts};
use dayzmake::api::steam::Branch;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Script and config text from a mod is read lossily, like the indexer does.
    let text = String::from_utf8_lossy(data);
    let files = [
        ScriptFile {
            module: 0,
            path: "3_Game\\a.c",
            text: &text,
        },
        ScriptFile {
            module: 0,
            path: "4_World\\b.c",
            text: &text,
        },
    ];
    let _ = collect_defines(&text, &files);
    let mut index = ApiIndex::new("fuzz", "fuzz", "fuzz");
    index_scripts(&files, &active_defines(Branch::Stable), &mut index);
});
