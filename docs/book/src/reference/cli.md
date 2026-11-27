# Command line

`dayzmake <command> [options] [arguments]`. The commands are in groups. Each group has its own page with one
section for each command.

| Group | Commands | Page |
|---|---|---|
| Configs | `rapify`, `preprocess`, `derapify` | [Configs](config.md) |
| Models | `binarize` | [Models](binarize.md) |
| PBOs | `build`, `pack`, `inspect`, `unpack`, `extract`, `cat` | [PBOs](pbo.md) |
| Keys and signatures | `keygen`, `sign`, `verify` | [Keys and signatures](signing.md) |
| Textures | `paa2img`, `img2paa` | [Textures](paa.md) |
| Script API | `api class`, `api find`, `api sub`, `api enum`, `api chain`, `api src`, `api grep`, `api defines`, `api update`, `api mods` | [Script API](api.md) |
| Navigation meshes | `navmesh probe`, `navmesh generate`, `navmesh info` | [Navigation meshes](navmesh.md) |
| MCP server | `mcp` | [MCP server](mcp.md) |

## Global options

| Option | Meaning |
|---|---|
| `-h`, `--help` | Print the usage text and exit with status 0. |
| `--version` | Print the version (for example `v3.2.0`) and exit with status 0. |

An unknown command or option prints `Invalid arguments.` followed by the usage text and exits with status 1.

## Options that many commands share

| Option | Meaning |
|---|---|
| `-v`, `--verbose` | Remove the limit of ten warnings for each warning name. `api` also prints a `# <index>: <build>` line for each index. |
| `-f`, `--force` | Overwrite the target if it exists. Without it, an existing target is an error: `Target file "x" already exists. Use --force to overwrite.` |
| `-w <name>`, `--warning <name>` | Mute the warning with this name. Repeatable. See [Warnings](warnings.md). |
| `-i <folder>`, `--include <folder>` | A folder that dayzmake searches for absolute includes. Repeatable. See [Config files and the preprocessor](../guides/configs.md). |

Only the commands that have `-w` or `-i` in their synopsis accept these options.

## Input and output

A command that takes `[<source>]` reads standard input when you omit it. A command that takes `[<target>]` writes
to standard output when you omit it. This works for `rapify`, `preprocess`, `derapify`, `inspect`, `cat`, `build`
and `pack` (the PBO goes to standard output). `build` and `pack` with `-k` need a real target, because dayzmake
cannot sign a stream: `error: Cannot sign a pbo that is piped to stdout.`

dayzmake writes targets atomically. It writes a temporary file next to the target and renames it when the command
succeeds. A failed command does not leave a half-written file.

## Exit status and messages

- `0`: the command succeeded. Warnings do not change the status.
- `1`: the command failed. The message is `error: <what failed>` on standard error. Invalid arguments also give 1.
- If a reader closes standard output early (`dayzmake inspect big.pbo | head`), the command ends quietly with
  status 0.

dayzmake writes warnings to standard error as `[In file <file>:<line>: ]warning: <message> [<name>]`. It shows a
warning name at most ten times. At the end, it gives a summary of the rest, with a hint to mute the warning with
`-w` or to use `-v`. dayzmake uses colours when standard output is a terminal (the messages themselves go to
standard error). `NO_COLOR=1` and `CLICOLOR=0` switch them off. `CLICOLOR_FORCE=1` forces them (see
[Environment and files](environment.md)).

## Help text

The usage text is the source of truth for the grammar. `dayzmake --help` prints it:

<div class="synopsis">

```text
dayzmake (-h | --help)
dayzmake --version
```

</div>
