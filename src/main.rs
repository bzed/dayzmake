use dayzmake::run;
use docopt::Docopt;

use crate::run::{Args, USAGE};

fn main() {
    let mut args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| e.exit());
    dayzmake::run::args(&mut args);
}
