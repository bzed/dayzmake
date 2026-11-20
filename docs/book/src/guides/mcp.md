# Using dayzmake from an MCP client

The Model Context Protocol (MCP) lets an AI assistant call tools on your machine. `dayzmake mcp` is an MCP server
that offers the script API index and the file commands of dayzmake as tools, so that an assistant can look up
real DayZ classes and members instead of guessing them, and can inspect, build and sign PBOs for you.

## Register the server

With Claude Code:

```text
$ claude mcp add dayzmake -- dayzmake mcp
```

The command after `--` is run by the client; it must be on the `PATH` of the client, or you give the full path
(`-- /home/me/.cargo/bin/dayzmake mcp`). For another MCP client, configure a server of the `stdio` kind with the
command `dayzmake` and the argument `mcp`. A typical JSON configuration looks like this:

```json
{
  "mcpServers": {
    "dayzmake": { "command": "dayzmake", "args": ["mcp"] }
  }
}
```

The server speaks newline-delimited JSON-RPC on standard input and output and ends when the client closes
them. It has no network access and needs no configuration.

## A safety note

The tools take file paths as given, relative to the folder the server was started in, and act with your
permissions. They can read your files, create files and, when the assistant passes `force`, overwrite them.
No tool overwrites anything without `force`, but `pbo_unpack`, `pbo_build` and the others do create files.
Register the server only with a client you trust to do that, and prefer to start the client in your project
folder.

## What to ask

The tools are described to the assistant, so ordinary requests work:

- "Which methods does `PlayerBase` have that are called `OnRPC`?" (`api_find`, `api_class`)
- "What do `Entity`'s subclasses override?" (`api_subclasses`, `api_class`)
- "Show me the values of `EStatTypes`." (`api_enum`)
- "Where is `Apple` declared? Show me the file." (`api_source`, with `cat`)
- "Which of my installed mods add to `PlayerBase`?" (`api_mods`, `api_class` with `mods` set to `all`)
- "What is in `mymod.pbo`? Is `config.bin` in it?" (`pbo_inspect`, `pbo_cat`)
- "Build the folder `mymod` and tell me the warnings." (`pbo_build`)
- "Sign `mymod.pbo` with `mykey.biprivatekey` and verify it." (`pbo_sign`, `pbo_verify`)
- "Rapify this config and show me what it turned into." (`config_rapify`, `config_derapify`)
- "Convert `a_co.paa` to PNG so I can look at it." (`paa_to_image`)
- "Build the navmesh of `mymap` from my capture `mymap.dzgeo` into `mymap.nm` and tell me how many tiles it has."
  (`navmesh_generate` with `capture` and `target`, then `navmesh_info`; a whole map from a live server takes many
  minutes, so ask for a `range` first)

The script API tools are vanilla-only by default. Say "including my mods" (the assistant passes `mods=all`) or
name a mod ("with CF") to include them. The first run with `mods=all` builds the caches and can take a while;
`api_update` with `mods=all` does it ahead of time.

## What the assistant sees

Each call returns text. Warnings come first (`warnings:` and one line each), then the output. Long output is cut
at 200000 bytes with a note that tells the assistant which `offset` continues. A failure is a result marked as
an error, so the assistant sees the message and can correct the call; the server keeps running.

A real exchange of one request and its reply, as the client sends it (see the [reference](../reference/mcp.md)
for a full session and every tool):

```text
> {"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"api_class","arguments":{"name":"PlayerBase","mods":"My Mod"}}}
{"id":2,"jsonrpc":"2.0","result":{"content":[{"text":"class PlayerBase : Entity  [vanilla 4_world]\n  bool IsAlive()  // Returns true while the player is alive.\n  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)\nclass PlayerBase  [My_Mod 4_world]  [modded]\n  void Heal(float amount)  // Added by MyMod.","type":"text"}],"isError":false}}
```

## Trying it by hand

You do not need a client to see what the server does:

```text
$ printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list"}' | dayzmake mcp | jq -r '.result.tools[].name'
api_class
api_find
…
```

(`jq` is only used to print the names.) Use `dayzmake mcp -v` to lift the ten-warnings-per-name limit in the tool
results.
