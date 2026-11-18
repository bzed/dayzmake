# Command line

`dayzmake <command> [options] [arguments]`. The commands fall into groups; each group has its own page with one
section per command.

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
| `--version` | Print the version (for example `v3.1.0`) and exit with status 0. |

An unknown command or option prints `Invalid arguments.` followed by the usage text and exits with status 1.

## Options that many commands share

| Option | Meaning |
|---|---|
| `-v`, `--verbose` | Lifts the limit of ten warnings per warning name. `api` also prints a `# <index>: <build>` line per index. |
| `-f`, `--force` | Overwrite the target if it exists. Without it, an existing target is an error: `Target file "x" already exists. Use --force to overwrite.` |
| `-w <name>`, `--warning <name>` | Mute the warning with this name. Repeatable. See [Warnings](warnings.md). |
| `-i <folder>`, `--include <folder>` | A folder that is searched for absolute includes. Repeatable. See [Config files and the preprocessor](../guides/configs.md). |

`-w` and `-i` are accepted only by the commands whose synopsis names them.

## Input and output

Commands that take `[<source>]` read standard input when it is omitted, and commands that take `[<target>]`
write standard output when it is omitted. This works for `rapify`, `preprocess`, `derapify`, `inspect`, `cat`,
`build` and `pack` (the PBO goes to standard output). `build` and `pack` with `-k` need a real target, because a
signature cannot be made of a stream: `error: Cannot sign a pbo that is piped to stdout.`

Targets are written atomically: dayzmake writes a temporary file next to the target and renames it when the
command succeeded, so a failed command leaves no half-written file.

## Exit status and messages

- `0`: the command succeeded. Warnings do not change the status.
- `1`: the command failed. The message is `error: <what failed>` on standard error. Invalid arguments also give 1.
- A reader that closes standard output early (`dayzmake inspect big.pbo | head`) ends the command quietly with
  status 0.

Warnings go to standard error as `[In file <file>:<line>: ]warning: <message> [<name>]`. A warning name is shown
at most ten times; the rest is summarised at the end with a hint to mute it with `-w` or to use `-v`. Colours are
used when standard output is a terminal (the messages themselves go to standard error); `NO_COLOR=1` and
`CLICOLOR=0` switch them off and `CLICOLOR_FORCE=1` forces them (see [Environment and files](environment.md)).

## Help text

The usage text is the source of truth for the grammar and is printed by `dayzmake --help`:

<div class="synopsis">

```text
dayzmake (-h | --help)
dayzmake --version
```

</div>
