use std::io::{Cursor, Error, Read, Write, stdin};
use std::path::{Path, PathBuf};

use crate::api;
use crate::api::query::Query;
use crate::api::steam::Branch;
use crate::cli_io::{AtomicTarget, Input, Output, StdoutState, open_input};
use crate::cli_warnings::{PrintError, PrintWarnings};
use crate::commands::{binarize, config, p3d, paa, pbo, sign};
use dayzmake_core::error;
use dayzmake_core::error::ErrorExt;
use dayzmake_core::p3d::model_magic;
use dayzmake_core::preprocess::IncludeFolders;
use dayzmake_core::sign::BISignVersion;
use dayzmake_core::warning::WarningSink;

use serde::Deserialize;

/// The docopt usage string: the CLI's `--help` text and command grammar.
pub const USAGE: &str = "
dayzmake

Usage:
    dayzmake rapify [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
    dayzmake preprocess [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
    dayzmake derapify [-v] [-f] [-d <indentation>] [<source> [<target>]]
    dayzmake binarize [-v] [-f] [-w <wname>]... <source> <target>
    dayzmake build [-v] [-f] [--proton-binarize] [-w <wname>]... [-i <includefolder>]... [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
    dayzmake pack [-v] [-f] [-x <excludepattern>]... [-e <headerext>]... [-k <privatekey>] [-s <signature>] <sourcefolder> [<target>]
    dayzmake inspect [-v] [<source>]
    dayzmake unpack [-v] [-f] [--use-prefix] <source> <targetfolder>
    dayzmake cat [-v] [-f] <source> <filename> [<target>]
    dayzmake keygen [-v] [-f] <keyname>
    dayzmake sign [-v] [-f] [--v2] <privatekey> <pbo> [<signature>]
    dayzmake paa2img [-v] [-f] <source> <target>
    dayzmake img2paa [-v] [-f] [-c] [-t <paatype>] <source> <target>
    dayzmake verify [-v] <publickey> <pbo> [<signature>]
    dayzmake api class [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--inherited] [--mods-only] [--json] <name>
    dayzmake api find [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--class <class>] [--json] <regex>
    dayzmake api sub [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--all] [--json] <name>
    dayzmake api enum [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] <name>
    dayzmake api chain [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--json] <name>
    dayzmake api src [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--cat] [--json] <name>
    dayzmake api defines [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] [<regex>]
    dayzmake api update [-v] [--experimental] [--dayz <dir>] [-m <mods>]
    dayzmake api mods [-v] [--experimental] [--dayz <dir>] [--json]
    dayzmake mcp [-v]
    dayzmake (-h | --help)
    dayzmake --version

Commands:
    rapify      Preprocess and rapify a config file.
    preprocess  Preprocess a file.
    derapify    Derapify a config.
    binarize    Binarize a file using BI's binarize.exe (Windows; on Linux through Proton).
                The model's folder is the addon and its name the prefix; use build
                for models whose textures are referenced by a longer prefix.
    build       Build a PBO from a folder.
    pack        Pack a folder into a PBO without any binarization or rapification.
    inspect     Inspect a PBO and list contained files, or a model (MLOD/ODOL) and
                show its format, version, LODs, textures, materials and selections.
    unpack      Unpack a PBO into a folder.
    cat         Read the named file from the target PBO to stdout.
    keygen      Generate a keypair with the specified path (extensions are added).
    sign        Sign a PBO with the given private key.
    verify      Verify a PBO's signature with the given public key.
    paa2img     Convert a PAA file to a standard image format (e.g. PNG).
    img2paa     Convert a standard image file to PAA.
    api         Query the DayZ script API from the installed scripts.pbo (vanilla) and,
                with -m, from mods: class, find, sub, enum, chain, src, defines, update;
                mods lists the installed mods.
    mcp         Run an MCP server on stdin/stdout (tools for the API index and the file commands).

Options:
    -v --verbose                Enable verbose output.
    -f --force                  Overwrite the target file/folder if it already exists.
    -w --warning <wname>        Warning to disable
    -i --include <includefolder>    Folder to search for includes; the CWD is only checked directly.
    -x --exclude <excludepattern>   Glob pattern to exclude from PBO.
                                      For unpack: pattern to exclude from output folder.
    -d --indent <indentation>   String to use for indentation. 4 spaces by default.
    -e --headerext <headerext>  Extension to add to PBO header as \"key=value\".
    -k --key <privatekey>       Sign the PBO with the given private key.
    -c --compress               Compress the output PAA file.
    -t --type <paatype>         PAA compression type (e.g. dxt1, dxt5). [default: dxt5]
    -s --signature <signature>  Signature path to use when signing the PBO.
    --v2                     Generate an older v2 signature.
    --use-prefix                unpack: extract into <targetfolder>/<prefix>/.
    --proton-binarize           Linux: binarize .p3d/.rtm with DayZ Tools' binarize.exe under Proton.
    --experimental              api: use the experimental branch instead of stable.
    --dayz <dir>                api: DayZ client/server directory (uses <dir>/dta/scripts.pbo).
    -m --mod <mods>             api: also query mods, comma separated: cf, cot, dabs, expansion, aliases
                                (those four, if installed), all (every installed mod with scripts; the
                                first run can take a while: each mod is indexed once, then cached), a
                                workshop id, a mod name or @dir name (see `api mods`) or a directory.
                                Without -m only vanilla is queried.
    --refresh                   api: rebuild the cached index.
    --max <n>                   api: print at most n lines (or JSON items). [default: 120]
    --inherited                 api class: also show the base classes.
    --mods-only                 api class: show only what the mods declare.
    --class <class>             api find: search only the members of this class or enum.
    --all                       api sub: include indirect subclasses.
    --cat                       api src: print the full source files.
    --json                      api: print JSON instead of text.
    -h --help                   Show usage information and exit.
       --version                Print the version number and exit.
";
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Parsed command-line arguments (populated by docopt from [`USAGE`]).
#[derive(Debug, Deserialize)]
#[expect(
    clippy::struct_excessive_bools,
    reason = "docopt requires one bool field per CLI flag"
)]
pub struct Args {
    /// Whether the `rapify` command was given.
    cmd_rapify: bool,
    cmd_preprocess: bool,
    cmd_derapify: bool,
    cmd_binarize: bool,
    cmd_build: bool,
    cmd_pack: bool,
    cmd_inspect: bool,
    cmd_unpack: bool,
    cmd_cat: bool,
    cmd_keygen: bool,
    cmd_sign: bool,
    cmd_paa2img: bool,
    cmd_img2paa: bool,
    cmd_verify: bool,
    cmd_api: bool,
    cmd_mcp: bool,
    cmd_class: bool,
    cmd_find: bool,
    cmd_sub: bool,
    cmd_enum: bool,
    cmd_chain: bool,
    cmd_src: bool,
    cmd_defines: bool,
    cmd_update: bool,
    cmd_mods: bool,
    flag_experimental: bool,
    flag_dayz: Option<String>,
    flag_mod: Option<String>,
    flag_refresh: bool,
    flag_max: String,
    flag_inherited: bool,
    flag_mods_only: bool,
    flag_class: Option<String>,
    flag_all: bool,
    flag_cat: bool,
    flag_json: bool,
    arg_name: String,
    arg_regex: Option<String>,
    flag_verbose: bool,
    flag_force: bool,
    flag_use_prefix: bool,
    flag_warning: Vec<String>,
    flag_include: Vec<String>,
    flag_exclude: Vec<String>,
    flag_headerext: Vec<String>,
    flag_key: Option<String>,
    flag_signature: Option<String>,
    flag_indent: Option<String>,
    flag_v2: bool,
    flag_proton_binarize: bool,
    flag_compress: bool,
    flag_type: String,
    flag_version: bool,
    arg_source: Option<String>,
    arg_target: Option<String>,
    arg_filename: String,
    arg_sourcefolder: String,
    arg_targetfolder: String,
    arg_keyname: String,
    arg_privatekey: String,
    arg_publickey: String,
    arg_signature: Option<String>,
    arg_pbo: String,
}

fn get_input(args: &Args) -> Result<Input, Error> {
    if let Some(ref source) = args.arg_source {
        Ok(Input::File(open_input(Path::new(source), "input file")?))
    } else {
        let mut buffer: Vec<u8> = Vec::new();
        stdin()
            .read_to_end(&mut buffer)
            .prepend_error("Failed to read from stdin:")?;
        Ok(Input::Cursor(Cursor::new(buffer.into_boxed_slice())))
    }
}

fn get_output(args: &Args, stdout: &StdoutState) -> Result<Output, Error> {
    if let Some(ref target) = args.arg_target {
        Ok(Output::File(AtomicTarget::create(
            Path::new(target),
            args.flag_force,
        )?))
    } else {
        Ok(Output::Standard(stdout.writer()))
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one branch per CLI command keeps the dispatch readable"
)]
fn run_command(
    args: &Args,
    stdout: &StdoutState,
    warnings: &mut dyn WarningSink,
) -> Result<(), Error> {
    // the working directory is only checked directly: it can be a whole disk (#66)
    let includefolders = IncludeFolders::new(args.flag_include.iter().map(PathBuf::from).collect())
        .with_direct(vec![PathBuf::from(".")]);

    if args.cmd_mcp {
        crate::mcp::serve(std::io::stdin().lock(), stdout.writer(), args.flag_verbose)
    } else if args.cmd_api {
        run_api(args, &mut stdout.writer(), warnings)
    } else if args.cmd_binarize {
        let source = required_source(args)?;
        let target = required_target(args)?;
        binarize::cmd_binarize(source, &target, args.flag_force, warnings)
    } else if args.cmd_rapify {
        let mut output = get_output(args, stdout)?;
        config::cmd_rapify(
            &mut get_input(args)?,
            &mut output,
            args.arg_source.as_ref().map(PathBuf::from),
            &includefolders,
            warnings,
        )?;
        output.commit()
    } else if args.cmd_derapify {
        let mut output = get_output(args, stdout)?;
        config::cmd_derapify(
            &mut get_input(args)?,
            &mut output,
            args.flag_indent.as_deref().unwrap_or("    "),
        )?;
        output.commit()
    } else if args.cmd_preprocess {
        let mut output = get_output(args, stdout)?;
        config::cmd_preprocess(
            &mut get_input(args)?,
            &mut output,
            args.arg_source.as_ref().map(PathBuf::from),
            &includefolders,
            warnings,
        )?;
        output.commit()
    } else if args.cmd_build || args.cmd_pack {
        let flag_privatekey = args.flag_key.as_ref().map(PathBuf::from);
        let flag_signature = args.flag_signature.as_ref().map(PathBuf::from);

        if flag_privatekey.is_some() && args.arg_target.is_none() {
            return Err(error!("Cannot sign a pbo that is piped to stdout."));
        }

        if args.cmd_build {
            // `close()` below removes the Proton sandbox on success and failure alike, before any exit
            // the target first: no sandbox is set up for a target that is rejected
            let mut output = get_output(args, stdout)?;
            let mut binarizer = platform_binarizer(args.flag_proton_binarize)?;
            let own = output.own_paths();
            let to_stdout = output.is_stdout();
            let built = pbo::cmd_build(
                Path::new(&args.arg_sourcefolder),
                &mut output,
                &args.flag_headerext,
                &args.flag_exclude,
                &includefolders,
                &own,
                to_stdout,
                binarizer.as_mut(),
                warnings,
            );
            // the build error wins over a cleanup error
            let closed = binarizer.map_or(Ok(()), |b| b.close(warnings));
            built?;
            closed?;
            output.commit()?;
        } else {
            let mut output = get_output(args, stdout)?;
            let own = output.own_paths();
            let to_stdout = output.is_stdout();
            pbo::cmd_pack(
                Path::new(&args.arg_sourcefolder),
                &mut output,
                &args.flag_headerext,
                &args.flag_exclude,
                &own,
                to_stdout,
                warnings,
            )?;
            output.commit()?;
        }

        if let Some(pkey) = flag_privatekey {
            let target = required_target(args)?;
            sign::cmd_sign(
                pkey,
                target,
                flag_signature,
                BISignVersion::V3,
                args.flag_force,
            )?;
        }

        Ok(())
    } else if args.cmd_inspect {
        let mut input = get_input(args)?;
        if model_magic(&mut input)?.is_some() {
            p3d::cmd_inspect(&mut input, &mut stdout.writer())
        } else {
            pbo::cmd_inspect(&mut input, &mut stdout.writer())
        }
    } else if args.cmd_cat {
        let mut output = get_output(args, stdout)?;
        pbo::cmd_cat(&mut get_input(args)?, &mut output, &args.arg_filename)?;
        output.commit()
    } else if args.cmd_unpack {
        pbo::cmd_unpack(
            &mut get_input(args)?,
            &PathBuf::from(&args.arg_targetfolder),
            args.flag_force,
            args.flag_use_prefix,
            warnings,
        )
    } else if args.cmd_keygen {
        sign::cmd_keygen(PathBuf::from(&args.arg_keyname), args.flag_force)
    } else if args.cmd_sign {
        let version = if args.flag_v2 {
            BISignVersion::V2
        } else {
            BISignVersion::V3
        };
        sign::cmd_sign(
            PathBuf::from(&args.arg_privatekey),
            PathBuf::from(&args.arg_pbo),
            args.arg_signature.as_ref().map(PathBuf::from),
            version,
            args.flag_force,
        )
    } else if args.cmd_paa2img {
        paa::cmd_paa2img(
            &required_source(args)?,
            &required_target(args)?,
            args.flag_force,
        )
    } else if args.cmd_img2paa {
        paa::cmd_img2paa(
            &required_source(args)?,
            &required_target(args)?,
            &args.flag_type,
            args.flag_compress,
            args.flag_force,
        )
    } else if args.cmd_verify {
        sign::cmd_verify(
            PathBuf::from(&args.arg_publickey),
            PathBuf::from(&args.arg_pbo),
            args.arg_signature.as_ref().map(PathBuf::from),
        )
    } else {
        Err(error!("no command given"))
    }
}

/// `dayzmake api ...`: load the indexes (rebuilding stale caches), run the query and print the answer.
fn run_api(args: &Args, out: &mut dyn Write, warnings: &mut dyn WarningSink) -> Result<(), Error> {
    let ctx = api::ApiContext {
        branch: if args.flag_experimental {
            Branch::Experimental
        } else {
            Branch::Stable
        },
        dayz: args.flag_dayz.as_ref().map(PathBuf::from),
        mods: args.flag_mod.clone().unwrap_or_default(),
        refresh: args.flag_refresh || args.cmd_update,
    };
    {
        if args.cmd_mods {
            let text = api::mods_text(&ctx, args.flag_json, warnings)?;
            if !text.is_empty() {
                writeln!(out, "{text}")?;
            }
            return Ok(());
        }
        if args.cmd_update {
            for line in api::update_lines(&ctx, warnings)? {
                writeln!(out, "{line}")?;
            }
            return Ok(());
        }
        let max: usize = args
            .flag_max
            .parse()
            .map_err(|_| error!("--max must be a number, not \"{}\"", args.flag_max))?;
        if args.cmd_src && args.flag_cat {
            let text = api::src_cat(&ctx, &args.arg_name, args.flag_json, warnings)?;
            if args.flag_json {
                writeln!(out, "{text}")?;
            } else {
                write!(out, "{text}")?;
            }
            return Ok(());
        }
        let name = args.arg_name.clone();
        let query = if args.cmd_class {
            Query::Class {
                name,
                inherited: args.flag_inherited,
                mods_only: args.flag_mods_only,
            }
        } else if args.cmd_find {
            Query::Find {
                regex: args.arg_regex.clone().unwrap_or_default(),
                class: args.flag_class.clone(),
            }
        } else if args.cmd_sub {
            Query::Sub {
                name,
                all: args.flag_all,
            }
        } else if args.cmd_enum {
            Query::Enum { name }
        } else if args.cmd_chain {
            Query::Chain { name }
        } else if args.cmd_src {
            Query::Src { name }
        } else if args.cmd_defines {
            Query::Defines {
                regex: args.arg_regex.clone(),
            }
        } else {
            return Err(error!("no api command given"));
        };
        for line in api::answer(
            &ctx,
            &query,
            max,
            args.flag_json,
            args.flag_verbose,
            warnings,
        )? {
            writeln!(out, "{line}")?;
        }
    }
    Ok(())
}

/// Returns the `<source>` argument, which docopt guarantees for these commands.
fn required_source(args: &Args) -> Result<PathBuf, Error> {
    args.arg_source
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| error!("missing <source> argument"))
}

/// Returns the `<target>` argument, which docopt guarantees for these commands.
fn required_target(args: &Args) -> Result<PathBuf, Error> {
    args.arg_target
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| error!("missing <target> argument"))
}

/// The binarizer for this platform: native on Windows, Proton on unix when `proton` is set,
/// otherwise none (models are then copied with a warning).
///
/// # Errors
///
/// Returns an error if the backend cannot be created.
pub fn platform_binarizer(
    proton: bool,
) -> Result<Option<dayzmake_core::binarize::Binarizer>, Error> {
    if cfg!(windows) {
        Ok(Some(dayzmake_core::binarize::Binarizer::new(
            dayzmake_core::binarize::BinarizeBackend::Native,
        )?))
    } else if proton {
        Ok(Some(dayzmake_core::binarize::Binarizer::new(
            dayzmake_core::binarize::BinarizeBackend::Proton,
        )?))
    } else {
        Ok(None)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn platform_binarizer_follows_the_proton_flag_on_unix() {
        assert!(platform_binarizer(false).unwrap().is_none());
        assert!(platform_binarizer(true).unwrap().is_some());
    }
}

/// Entry point driving the CLI: handles `--version`, then dispatches to the command.
pub fn args(args: &mut Args) {
    if args.flag_indent.is_none() {
        args.flag_indent = Some("    ".to_string());
    }

    if args.flag_version {
        // a closed pipe is not worth a message here either
        let _ = writeln!(StdoutState::default().writer(), "v{VERSION}");
        std::process::exit(0);
    }

    let mut warnings = PrintWarnings::new(args.flag_warning.clone(), args.flag_verbose);
    let stdout = StdoutState::default();
    let result = run_command(args, &stdout, &mut warnings);
    warnings.print_summary();
    // The reader closed stdout (`dayzmake inspect x.pbo | head`): like other tools, stop quietly with status 0.
    // Only a failed write to stdout counts; a BrokenPipe on stderr, a target file or a tool is still an error.
    if result.is_err() && stdout.is_closed() {
        std::process::exit(0);
    }
    result.print_error(true);
}
