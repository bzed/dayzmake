# MCP server: mcp and its tools

The Model Context Protocol (MCP) lets an AI assistant call tools. `dayzmake mcp` is an MCP server that offers the
script API index and the file commands of dayzmake as tools. The [guide](../guides/mcp.md) shows how to register
it; this page is the reference of the protocol subset and of every tool.

## dayzmake mcp

Run the MCP server on standard input and output.

<div class="synopsis">

```text
dayzmake mcp [-v]
```

</div>

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Lift the limit of ten warnings per name in each tool call. |

Behaviour:

- Transport: newline-delimited JSON-RPC 2.0 on standard input and output, one JSON object per line. Standard
  output carries nothing but JSON-RPC replies; progress notes of tools (for example the Proton sandbox) go to
  standard error. The server runs until its standard input ends and then exits with status 0.
- Methods: `initialize` (answers with the protocol version it was asked for if it is `2025-06-18`, `2025-03-26`
  or `2024-11-05`, else `2025-06-18`; capabilities are `{"tools": {}}`), `ping`, `tools/list` and `tools/call`.
  Notifications get no reply. Resources, prompts and sampling are not offered.
- Requests are handled one after the other. A request line may be at most 16 MiB; a longer one is answered with
  `request line too long` and skipped.
- Tool arguments are JSON objects; a missing required argument, or an argument of the wrong type, gives a tool
  error result such as `argument "pbo" must be a string`.
- The tools take file paths as given, relative to the working directory of the server, and act with the
  permissions of the user who runs it: they can read, create and, with `force`, overwrite any file that user can.
  No tool overwrites a file unless `force` is true. Register the server only with a client you trust to do that.

See also: [Using dayzmake from an MCP client](../guides/mcp.md), [Script API: api](api.md).

## Results and errors

A successful `tools/call` returns `{"content": [{"type": "text", "text": "..."}], "isError": false}`. A tool that
fails returns the same shape with `isError: true` and the error message as the text; the server keeps running.

- Warnings of the call come first: `warnings:` and one line per warning, a blank line, then the output. A cut
  never drops them. Each call has its own ten-per-name limit.
- Output is capped at 200000 bytes. A cut result ends with `[truncated: showing bytes A-B of N; call again with
  offset=B]`. The `offset` and `length` arguments exist on `pbo_cat` and `api_source`; other tools return their
  whole (short) output.
- A panic in a tool is caught and reported as `internal error: the tool "<name>" panicked`.

Protocol errors are JSON-RPC errors:

| Code | When |
|---|---|
| `-32700` | The line is not JSON (`parse error`). |
| `-32600` | The request is not an object, has an invalid `id` or no `method`, or the line is too long. |
| `-32601` | Unknown method (`method not found: <method>`). |
| `-32602` | `tools/call` with bad parameters or an unknown tool (`unknown tool "<name>"`). |

A session with the real server (the lines starting with `>` are requests, the other lines replies; `mymod.pbo`
is the PBO of the [first mod guide](../guides/first-mod.md)):

```text
> {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18","capabilities":{},"clientInfo":{"name":"doc","version":"1"}}}
{"id":1,"jsonrpc":"2.0","result":{"capabilities":{"tools":{}},"protocolVersion":"2025-06-18","serverInfo":{"name":"dayzmake","version":"3.0.2"}}}
> {"jsonrpc":"2.0","method":"notifications/initialized"}
> {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"api_class","arguments":{"name":"PlayerBase","mods":"My Mod"}}}
{"id":2,"jsonrpc":"2.0","result":{"content":[{"text":"class PlayerBase : Entity  [vanilla 4_world]\n  bool IsAlive()  // Returns true while the player is alive.\n  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)\nclass PlayerBase  [My_Mod 4_world]  [modded]\n  void Heal(float amount)  // Added by MyMod.","type":"text"}],"isError":false}}
> {"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"pbo_cat","arguments":{"pbo":"mymod.pbo","file":"config.bin"}}}
{"id":3,"jsonrpc":"2.0","result":{"content":[{"text":"the entry is not UTF-8 text (291 bytes); pass target to write it to a file","type":"text"}],"isError":true}}
> {"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"nope","arguments":{}}}
{"error":{"code":-32602,"message":"unknown tool \"nope\""},"id":4,"jsonrpc":"2.0"}
> {"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"pbo_build","arguments":{"source":"t21","target":"t21-mcp.pbo"}}}
{"id":5,"jsonrpc":"2.0","result":{"content":[{"text":"warnings:\nIn file small.p3d: warning: Model references texture \"t21\\data\\gone_co.paa\", which is not in the addon. [missing-texture]\nIn file small.p3d: warning: Model references material \"t21\\data\\m.rvmat\", which is not in the addon. [missing-material]\nIn file small.p3d: warning: On non-Windows systems binarize.exe is only used with --proton-binarize; file will be copied as-is. [non-windows-binarization]\n\nwrote t21-mcp.pbo","type":"text"}],"isError":false}}
> {"jsonrpc":"2.0","id":6,"method":"tools/call","params":{"name":"pbo_build","arguments":{"source":"t21","target":"t21-mcp.pbo"}}}
{"id":6,"jsonrpc":"2.0","result":{"content":[{"text":"Target file \"t21-mcp.pbo\" already exists. Use --force to overwrite.","type":"text"}],"isError":true}}
> {"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"pbo_verify","arguments":{"public_key":"mykey.bikey","pbo":"mymod.pbo"}}}
{"id":7,"jsonrpc":"2.0","result":{"content":[{"text":"signature is valid","type":"text"}],"isError":false}}
```

## Tools

The API tools take the same `mods`, `experimental`, `dayz` and `refresh` arguments, and `json` and `max` where the
CLI command has `--json` and `--max`; without `mods` only vanilla is queried and no tool defaults to `all`. The
file tools take the arguments of their CLI counterparts, with `force` for `--force`.

### api_class

Members of a DayZ script class from the installed scripts.pbo (vanilla; mods only when selected with mods).

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Class name. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `inherited` | boolean | no | Also show the base classes. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `mods_only` | boolean | no | Show only what the selected mods declare. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: The text of `dayzmake api class`, or JSON with `json`. See [api class](api.md#dayzmake-api-class).

### api_find

Case-insensitive regex over member names, signatures, docs, class names and enum items (so ^OnRPC finds the OnRPC methods).

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `regex` | string | yes | The regular expression. |
| `class` | string | no | Search only the members of this class or enum. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: The text of `dayzmake api find`. See [api find](api.md#dayzmake-api-find).

### api_subclasses

Subclasses of a class.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Class name. |
| `all` | boolean | no | Include indirect subclasses. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: The names on one line, as `dayzmake api sub`.

### api_enum

Items of an enum.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Enum name. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: The text of `dayzmake api enum`.

### api_chain

Base class chain of a class.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Class name. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: The chain, as `dayzmake api chain`.

### api_source

Files and lines that declare a class or enum; with cat the full source files, with context or lines part of them.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Class or enum name. |
| `cat` | boolean | no | Return the full source files (with json: the files as JSON). |
| `context` | integer (&ge; 0) | no | Return n lines around each declaration, with line numbers (excludes cat and lines). |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `force` | boolean | no | Overwrite existing target files. |
| `json` | boolean | no | Return JSON instead of text. |
| `length` | integer (&ge; 0) | no | Return at most this many bytes (default and maximum 200000). |
| `lines` | string | no | Return lines a to b of each file as "a-b", with line numbers (excludes cat and context). |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `offset` | integer (&ge; 0) | no | Start at this byte of the output (default 0). A cut-off reply names the offset that fetches the rest. |
| `refresh` | boolean | no | Rebuild the cached index first. |
| `target` | string | no | With cat: write the files to this file instead of returning them (cannot be combined with context or lines). |

Result: The locations, as `dayzmake api src`; with `cat` the source files, which can be long: use `offset` and `length` to page, or `target` to write them to a file (`target` together with `context` or `lines` is an error).

### api_grep

Full-text search over the script bodies (every .c file of vanilla and the selected mods, read from the PBOs). Matches raw lines, comments included.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `regex` | string | yes | Regular expression, matched against each line. |
| `ignore_case` | boolean | no | Match case-insensitively. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |

Result: The matching lines, as `dayzmake api grep`.

### api_config

Look up a config class (config.bin and config.cpp of the game and the selected mods, merged in load order the way the engine does): its inheritance chain, its own and inherited properties each tagged with the class they come from, its direct subclasses and the PBOs that define it; a redefinition that changes or drops the base class (as the engine does) is reported as a base change. The path is a /-separated class path such as CfgVehicles/Hatchback_02, matched ignoring case.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `path` | string | yes | The class path, for example CfgVehicles/Hatchback_02 or CfgWeapons/M4A1. |
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: The class, as `dayzmake api config`.

### api_defines

Compile-time defines of the selected indexes, optionally filtered by a regex.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |
| `max` | integer (&ge; 0) | no | At most this many lines or items (default 120). |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |
| `regex` | string | no | Filter regular expression. |

Result: The defines, as `dayzmake api defines`.

### api_update

Rebuild the cached indexes now. With mods=all this indexes every installed mod, which warms the caches for later queries.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `dayz` | string | no | DayZ client/server directory (uses `<dir>`/dta/scripts.pbo) instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `mods` | string | no | Also query these mods, comma separated: cf, cot, dabs, expansion, aliases (those four, if installed), all (every installed mod with scripts; the first run builds the caches and can take a while, later runs are quick; api_update with mods=all warms them), a workshop id, a mod name or @dir name (see api_mods) or a mod directory. Without it only vanilla is queried. |
| `refresh` | boolean | no | Rebuild the cached index first. |

Result: One line per rebuilt index, as `dayzmake api update`.

### api_mods

List the installed mods (workshop id, name, directory, whether they have scripts); the names and ids can be used in the mods argument of the other api tools. Reading them is quick; querying all of them with mods=all builds the caches on the first run and can take a while; api_update with mods=all warms them.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `dayz` | string | no | DayZ client/server directory instead of the Steam install. |
| `experimental` | boolean | no | Use the experimental branch instead of stable. |
| `json` | boolean | no | Return JSON instead of text. |

Result: The installed mods, as `dayzmake api mods` (JSON with `json`).

### pbo_inspect

List the entries of a PBO, or describe a model (P3D, MLOD or ODOL v56: format, LODs, textures, selections).

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `path` | string | yes | PBO or P3D file. |
| `json` | boolean | no | PBOs only: return the header as JSON with the raw bytes of names as hex. |

Result: The text of `dayzmake inspect`: the PBO listing or the model description; with `json`, the object of
`dayzmake inspect --json` (an error for a model).

### pbo_cat

Read one entry of a PBO: returned as text if it is UTF-8, otherwise write it to target.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `file` | string | yes | Entry name. |
| `pbo` | string | yes | PBO file. |
| `force` | boolean | no | Overwrite existing target files. |
| `length` | integer (&ge; 0) | no | Return at most this many bytes (default and maximum 200000). |
| `offset` | integer (&ge; 0) | no | Start at this byte of the output (default 0). A cut-off reply names the offset that fetches the rest. |
| `target` | string | no | Write the entry to this file instead of returning it (required for binary data). |

Result: The entry as text when it is UTF-8, or `wrote <target> (<n> bytes)` with `target`. A binary entry without `target` is an error: `the entry is not UTF-8 text (291 bytes); pass target to write it to a file`. Large entries are cut at 200000 bytes with a note that names the `offset` that continues.

### pbo_unpack

Unpack a PBO into a folder, all of it or the entries selected by `only` and `exclude`. Unsafe entry names are skipped with a warning.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `pbo` | string | yes | PBO file. |
| `target` | string | yes | Folder to extract into. |
| `force` | boolean | no | Overwrite existing target files. |
| `use_prefix` | boolean | no | Extract into `<target>`/`<prefix>`/. |
| `exclude` | array of strings | no | Glob patterns of entries to leave out (case-insensitive, `/` separators). |
| `only` | array of strings | no | Glob patterns: write only the entries that match one (`exclude` wins). |

Result: `unpacked <pbo> to <target>`.

### extract

Unpack every PBO of a DayZ install, a mod folder or a single PBO into one tree `<target>`/`<prefix>`/`<entry>`, optionally derapified, and write the prefix to PBO map `pbo-map.tsv`. The tree replaces an existing target atomically with `force`.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | DayZ client or server folder, mod folder (`dta/` and `addons/` are searched) or one PBO. |
| `target` | string | yes | Folder to write the tree to. |
| `force` | boolean | no | Replace an earlier extract result or an empty folder. |
| `derapify` | boolean | no | Write every rapified `.bin` as a `.cpp` of the same name. |
| `exclude` | array of strings | no | Glob patterns of entries to leave out (case-insensitive, `/` separators). |
| `only` | array of strings | no | Glob patterns: write only the entries that match one (`exclude` wins). |

Result: `extracted <n> files from <m> PBOs to <target>`. See [extract](pbo.md#dayzmake-extract).

### pbo_pack

Pack a folder into a PBO without binarizing or rapifying.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | Folder to pack. |
| `target` | string | yes | PBO file to write. |
| `exclude` | array of strings | no | Glob patterns to leave out. |
| `force` | boolean | no | Overwrite existing target files. |
| `headerext` | array of strings | no | Header extensions as key=value. |
| `key` | string | no | Private key (.biprivatekey): sign the PBO after packing. |

Result: `wrote <target>`, or `wrote and signed <target>` when `key` was given.

### pbo_build

Build a PBO from a folder, rapifying configs and binarizing models.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | Folder to pack. |
| `target` | string | yes | PBO file to write. |
| `exclude` | array of strings | no | Glob patterns to leave out. |
| `force` | boolean | no | Overwrite existing target files. |
| `headerext` | array of strings | no | Header extensions as key=value. |
| `include` | array of strings | no | Folders to search for includes (the current directory is only checked directly, not searched). |
| `key` | string | no | Private key (.biprivatekey): sign the PBO after packing. |
| `proton_binarize` | boolean | no | Linux: binarize models with DayZ Tools' binarize.exe under Proton. |

Result: `wrote <target>`. Warnings of the build come first in the text.

### config_rapify

Preprocess and rapify a config file.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | Config source file. |
| `target` | string | yes | File to write. |
| `force` | boolean | no | Overwrite existing target files. |
| `include` | array of strings | no | Folders to search for includes. |

Result: `wrote <target> (<n> bytes)`.

### config_derapify

Convert a rapified config back to text.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | Rapified config. |
| `force` | boolean | no | Overwrite existing target files. |
| `target` | string | no | Write the text here; absent: return it. |
| `indent` | string | no | Indentation per level: spaces and tabs only, at most 64 characters, default four spaces. |

Result: The text of the config, or `wrote <target> (<n> bytes)` with `target`.

### config_preprocess

Preprocess a file (macros, includes).

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | File to preprocess. |
| `force` | boolean | no | Overwrite existing target files. |
| `include` | array of strings | no | Folders to search for includes. |
| `target` | string | no | Write the result here; absent: return it. |

Result: The preprocessed text, or `wrote <target> (<n> bytes)` with `target`.

### paa_to_image

Convert a PAA texture to an image (the format follows the target extension, e.g. png). DXT1 to DXT5, RGBA,
ARGB4444, ARGB1555 and AI88 are read; a PNG file named `.paa` gets an error that says so.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | PAA file. |
| `target` | string | yes | Image file to write. |
| `force` | boolean | no | Overwrite existing target files. |

Result: `wrote <target>`.

### image_to_paa

Convert an image to a PAA texture.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | Image file. |
| `target` | string | yes | PAA file to write. |
| `compress` | boolean | no | LZO-compress DXT mipmaps wider than 128 px (always done; kept for compatibility). |
| `force` | boolean | no | Overwrite existing target files. |
| `type` | string | no | PAA type, e.g. dxt1, dxt5 (default dxt5). |

Result: `wrote <target>`.

### key_generate

Generate a key pair (`<name>.biprivatekey` and `<name>.bikey`).

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `name` | string | yes | Path of the key without extension. |
| `force` | boolean | no | Overwrite existing target files. |

Result: `generated <name>.biprivatekey and <name>.bikey`.

### pbo_sign

Sign a PBO with a private key.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `pbo` | string | yes | PBO file. |
| `private_key` | string | yes | Private key file. |
| `force` | boolean | no | Overwrite existing target files. |
| `signature` | string | no | Signature file (default `<pbo>.<key>.bisign`). |
| `v2` | boolean | no | Write an older v2 signature. |

Result: `signed`; the signature is written to `signature`, by default `<pbo>.<key>.bisign`.

### pbo_verify

Verify the signature of a PBO with a public key.

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `pbo` | string | yes | PBO file. |
| `public_key` | string | yes | Public key file (.bikey). |
| `signature` | string | no | Signature file (default `<pbo>.<key>.bisign`). |

Result: `signature is valid`, or an error result that says what does not match.

### binarize

Binarize a model with BI's binarize.exe (Windows; on Linux through Proton).

| Argument | Type | Required | Meaning |
|---|---|---|---|
| `source` | string | yes | Model file. |
| `target` | string | yes | File to write. |
| `force` | boolean | no | Overwrite existing target files. |

Result: `wrote <target>`, or an error when DayZ Tools or Proton cannot be found (see [Binarizing models](binarize.md)).
