//! The dayzmake binary: CLI entry point.

use std::io::{self, Write};

use dayzmake::cli_io::StdoutState;
use dayzmake::run;
use docopt::{Docopt, Error};

use crate::run::{Args, USAGE};

/// Prints a usage error to stderr (status 1) or help/version to stdout (status 0), like docopt's own
/// `Error::exit`, but without its `unwrap` on the write.
fn exit_with(e: &Error) -> ! {
    if e.fatal() {
        // a failed write to stderr (the reader is gone) cannot be reported anywhere; docopt would panic
        let _ = writeln!(io::stderr(), "{e}");
        std::process::exit(1)
    }
    // a closed stdout ends quietly with status 0 either way
    let _ = writeln!(StdoutState::default().writer(), "{e}");
    std::process::exit(0)
}

fn main() {
    let mut args: Args = Docopt::new(USAGE)
        .and_then(|d| d.deserialize())
        .unwrap_or_else(|e| exit_with(&e));
    dayzmake::run::args(&mut args);
}
