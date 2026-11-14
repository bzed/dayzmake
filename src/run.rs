use std::fs::File;
use std::io::{Cursor, Error, Read, stdin, stdout};
use std::path::PathBuf;

use crate::api;
use crate::api::query::{Query, render_json, render_text, run_query};
use crate::api::steam::Branch;
use crate::binarize;
use crate::config;
use crate::error;
use crate::error::{ErrorExt, print_warning_summary};
use crate::io::{Input, Output};
use crate::p3d;
use crate::paa;
use crate::pbo;
use crate::preprocess;
use crate::sign;

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
    dayzmake cat [-v] <source> <filename> [<target>]
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
                with -m, from mods: class, find, sub, enum, chain, src, defines, update.

Options:
    -v --verbose                Enable verbose output.
    -f --force                  Overwrite the target file/folder if it already exists.
    -w --warning <wname>        Warning to disable
    -i --include <includefolder>    Folder to search for includes, defaults to CWD.
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
    -m --mod <mods>             api: also query mods: cf, cot, dabs, expansion, all (those installed),
                                a workshop id or a mod directory; comma separated.
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
    cmd_class: bool,
    cmd_find: bool,
    cmd_sub: bool,
    cmd_enum: bool,
    cmd_chain: bool,
    cmd_src: bool,
    cmd_defines: bool,
    cmd_update: bool,
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
        Ok(Input::File(
            File::open(source).prepend_error("Failed to open input file:")?,
        ))
    } else {
        let mut buffer: Vec<u8> = Vec::new();
        stdin()
            .read_to_end(&mut buffer)
            .prepend_error("Failed to read from stdin:")?;
        Ok(Input::Cursor(Cursor::new(buffer.into_boxed_slice())))
    }
}

fn get_output(args: &Args) -> Result<Output, Error> {
    if let Some(ref target) = args.arg_target {
        let path = PathBuf::from(target);
        if !args.flag_force && path.exists() {
            return Err(error!(
                "Target file \"{}\" already exists. Use --force to overwrite.",
                target
            ));
        }
        Ok(Output::File(
            File::create(path).prepend_error("Failed to open output file:")?,
        ))
    } else {
        Ok(Output::Standard(stdout()))
    }
}

#[expect(
    clippy::too_many_lines,
    reason = "one branch per CLI command keeps the dispatch readable"
)]
fn run_command(args: &Args) -> Result<(), Error> {
    let mut includefolders: Vec<PathBuf> = args.flag_include.iter().map(PathBuf::from).collect();
    includefolders.push(PathBuf::from("."));

    if args.cmd_api {
        run_api(args)
    } else if args.cmd_binarize {
        let source = required_source(args)?;
        let target = required_target(args)?;
        binarize::cmd_binarize(source, &target, args.flag_force)
    } else if args.cmd_rapify {
        config::cmd_rapify(
            &mut get_input(args)?,
            &mut get_output(args)?,
            args.arg_source.as_ref().map(PathBuf::from),
            &includefolders,
        )
    } else if args.cmd_derapify {
        config::cmd_derapify(&mut get_input(args)?, &mut get_output(args)?)
    } else if args.cmd_preprocess {
        preprocess::cmd_preprocess(
            &mut get_input(args)?,
            &mut get_output(args)?,
            args.arg_source.as_ref().map(PathBuf::from),
            &includefolders,
        )
    } else if args.cmd_build || args.cmd_pack {
        let flag_privatekey = args.flag_key.as_ref().map(PathBuf::from);
        let flag_signature = args.flag_signature.as_ref().map(PathBuf::from);

        if flag_privatekey.is_some() && args.arg_target.is_none() {
            return Err(error!("Cannot sign a pbo that is piped to stdout."));
        }

        if args.cmd_build {
            pbo::cmd_build(
                PathBuf::from(&args.arg_sourcefolder),
                &mut get_output(args)?,
                &args.flag_headerext,
                &args.flag_exclude,
                &includefolders,
            )?;
        } else {
            pbo::cmd_pack(
                PathBuf::from(&args.arg_sourcefolder),
                &mut get_output(args)?,
                &args.flag_headerext,
                &args.flag_exclude,
            )?;
        }

        if let Some(pkey) = flag_privatekey {
            let target = required_target(args)?;
            sign::cmd_sign(
                pkey,
                target,
                flag_signature,
                sign::BISignVersion::V3,
                args.flag_force,
            )?;
        }

        Ok(())
    } else if args.cmd_inspect {
        let mut input = get_input(args)?;
        if p3d::model_magic(&mut input)?.is_some() {
            p3d::cmd_inspect(&mut input)
        } else {
            pbo::cmd_inspect(&mut input)
        }
    } else if args.cmd_cat {
        pbo::cmd_cat(
            &mut get_input(args)?,
            &mut get_output(args)?,
            &args.arg_filename,
        )
    } else if args.cmd_unpack {
        pbo::cmd_unpack(
            &mut get_input(args)?,
            &PathBuf::from(&args.arg_targetfolder),
            args.flag_force,
            args.flag_use_prefix,
        )
    } else if args.cmd_keygen {
        sign::cmd_keygen(PathBuf::from(&args.arg_keyname), args.flag_force)
    } else if args.cmd_sign {
        let version = if args.flag_v2 {
            sign::BISignVersion::V2
        } else {
            sign::BISignVersion::V3
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
fn run_api(args: &Args) -> Result<(), Error> {
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
    #[expect(clippy::print_stdout, reason = "CLI command output")]
    {
        if args.cmd_update {
            for (key, index) in api::load_keyed(&ctx)? {
                println!("{key}: {} classes, {}", index.classes.len(), index.build);
            }
            return Ok(());
        }
        let max: usize = args
            .flag_max
            .parse()
            .map_err(|_| error!("--max must be a number, not \"{}\"", args.flag_max))?;
        if args.cmd_src && args.flag_cat {
            if args.flag_json {
                let files = api::src_contents(&ctx, &args.arg_name)?;
                let json = serde_json::to_string(&serde_json::json!({ "files": files }))
                    .map_err(|e| error!("cannot serialize: {e}"))?;
                println!("{json}");
            } else {
                print!("{}", api::cat_sources(&ctx, &args.arg_name)?);
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
        let indexes = api::load_indexes(&ctx)?;
        let result = run_query(&indexes, &query)?;
        if args.flag_json {
            println!("{}", render_json(&result, max));
        } else {
            if args.flag_verbose {
                for i in &indexes {
                    println!("# {}: {}", i.name, i.build);
                }
            }
            for line in render_text(&result, indexes.len() > 1, max) {
                println!("{line}");
            }
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

/// Entry point driving the CLI: handles `--version`, then dispatches to the command.
pub fn args(args: &mut Args) {
    if args.flag_indent.is_none() {
        args.flag_indent = Some("    ".to_string());
    }

    if args.flag_version {
        #[expect(clippy::print_stdout, reason = "CLI command output")]
        {
            println!("v{VERSION}");
        }
        std::process::exit(0);
    }

    binarize::set_proton(args.flag_proton_binarize);
    error::init_warnings(args.flag_warning.clone(), args.flag_verbose);
    let result = run_command(args);
    binarize::cleanup(); // must happen before print_error, which exits on error
    result.print_error(true);

    print_warning_summary();
}
