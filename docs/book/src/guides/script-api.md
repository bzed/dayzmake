# Looking up the script API

DayZ scripts (Enforce Script) have no official reference that stays current. `dayzmake api` reads the scripts of
the game you have installed, `dta/scripts.pbo`, indexes the classes, members and enums once and answers lookups in
milliseconds, without extracting anything. It can also index the scripts of mods, so that you see vanilla and
mod declarations side by side.

The examples use a tiny stand-in game so that the output is short and reproducible. It has `Entity`, `ItemBase`,
`PlayerBase` and `Apple`, the enum `EStatTypes`, and the stand-in is selected with `--dayz dz`. With your real
Steam install leave `--dayz` out: dayzmake finds the game itself (see [Environment and files](../reference/environment.md)).

## Who calls this?

`api find` matches declarations only. To find call sites and other usages, search the script bodies with
`api grep`; the lines are matched as they are in the source, comments included:

```text
$ dayzmake api grep 'GetGame\(\)\.GetMission\(\)' --max 2
scripts\3_Game\Sandstorm.c:95: \t\t\tGetGame().GetMission().GetWorldData().WeatherSandstormStart(continuedSession);
scripts\3_Game\Sandstorm.c:115: \t\t\tGetGame().GetMission().GetWorldData().WeatherSandstormStop(calledByWeather);
... 4 more lines (raise --max or narrow the query)
```

Add `--ignore-case` for a case-insensitive match and `-m cf` to search a mod's scripts too. See
[api grep](../reference/api.md#dayzmake-api-grep).

## Reading the game sources

For everything the index does not answer, extract the sources once and use your editor and `grep`. `extract` unpacks
every PBO of an install under its prefix and derapifies the configs, so `dz\scripts` ends up in `dz/scripts`
and every `config.bin` is a `config.cpp`:

```text
$ dayzmake extract --derapify --only '*.c' --only '*.bin' --only '*.cpp' --only '*.h' ~/DayZ game-sources
```

`game-sources/pbo-map.tsv` tells which PBO a prefix came from. Run it again with `-f` after a game update: the old
tree is replaced in one step and files that no longer exist are gone. A mod folder (`@Foo`) or a single PBO works
the same. See [extract](../reference/pbo.md#dayzmake-extract).

## What does this class have?

```text
$ dayzmake api --dayz dz class PlayerBase
class PlayerBase : Entity
  bool IsAlive()  // Returns true while the player is alive.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
```

The first line is the class and its base. Members are sorted; documentation comments (`//!`) from the source
follow the member. `class` shows the class itself; add `--inherited` to see the members of the base classes too,
and `--max` to see more than 120 lines.

```text
$ dayzmake api --dayz dz class --inherited ItemBase
class ItemBase : Entity
  int m_Quantity
  static const int MAX_STACK=100
  int GetQuantity()  // Quantity of the item in its stack.
  void SetQuantity(int value,bool destroy_config=true)
class Entity
  protected float m_Health
  float GetHealth()  // Current health of the entity.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
  void SetHealth(float value)
```

## Where is it, and what derives from it?

```text
$ dayzmake api --dayz dz chain Apple
Apple > ItemBase > Entity
$ dayzmake api --dayz dz sub Entity
ItemBase PlayerBase
$ dayzmake api --dayz dz sub --all Entity
ItemBase PlayerBase Apple
$ dayzmake api --dayz dz src PlayerBase
[vanilla] dta/scripts.pbo scripts\4_world\player.c:1
$ dayzmake api --dayz dz src --cat Apple
// ==== [vanilla] dta/scripts.pbo scripts\4_world\player.c:8
class PlayerBase extends Entity
…
```

`src` tells you the PBO, the path inside it and the line; `src --cat` prints the files, with a header line for
each, so you can read the code around a declaration.

## Search

`find` takes a case-insensitive regular expression and searches member names, signatures, doc comments, class
names and enum items. Anchor it to look for names:

```text
$ dayzmake api --dayz dz find '^OnRPC'
Entity: void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
PlayerBase: void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
$ dayzmake api --dayz dz find --class ItemBase quantity
ItemBase: int m_Quantity
ItemBase: int GetQuantity()  // Quantity of the item in its stack.
ItemBase: void SetQuantity(int value,bool destroy_config=true)
```

`enum` shows the values of an enum, computed the way the compiler does (an item without a value continues from
the previous one):

```text
$ dayzmake api --dayz dz enum EStatTypes
enum EStatTypes
  HEALTH = 0
  BLOOD = 1
  SHOCK = 10
  WATER = 11
```

## Adding mods

A mod is selected with `-m`. Without it only vanilla is queried, so a query never mixes in mods by surprise.

```text
$ dayzmake api --dayz dz class -m @MyMod PlayerBase
class PlayerBase : Entity  [vanilla 4_world]
  bool IsAlive()  // Returns true while the player is alive.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
class PlayerBase  [_MyMod 4_world]  [modded]
  void Heal(float amount)  // Added by MyMod.
```

The mod's `modded class` adds `Heal`. `--mods-only` shows only the mod's part. What `-m` accepts:

- a folder, such as `@MyMod` above (it needs an `addons` folder with PBOs);
- a Steam Workshop id, or the name of an installed mod (`-m "my mod"`, a unique part is enough);
- the aliases `cf`, `cot`, `dabs`, `expansion` for the well-known frameworks, or `aliases` for all four;
- `all` for every installed mod that has scripts.

`dayzmake api mods` lists what is installed:

```text
$ dayzmake api mods
1234567890  My Mod  …/steamapps/workshop/content/221100/1234567890  scripts
```

The first query with a mod indexes its PBOs, which can take a while for big mods; the result is cached, and a
query with `-m all` indexes each mod only once. To pay that cost ahead of time, for example after a game update:

```text
$ dayzmake api -m all update
vanilla-stable: 4 classes, stable build 17000000
mod-My_Mod-stable-ceb44f75: 2 classes, stable
```

Compile-time defines of mods (the `defines[]` of `CfgMods` and `#define` in scripts) are listed with `defines`:

```text
$ dayzmake api --dayz dz defines -m @MyMod 'MYMOD|SERVER'
[_MyMod] mymod: MYMOD
```

## Branches

The stable branch is the default. `--experimental` queries the experimental branch, whose install the same Steam
libraries are searched for; if it is not installed you get an error that tells you to pass `--dayz <dir>`.

## JSON for scripts and tools

Every query takes `--json`, and then prints one line of JSON: `{"result": {...}, "truncated": N}`. Pipe it into
`jq`:

```text
$ dayzmake api --dayz dz find --json OnRPC | jq -r '.result.Matches[] | [.scope, .member.decl.path, .member.decl.line] | @tsv'
Entity	3_Game\\entity.c	15
PlayerBase	4_world\\player.c	5
$ dayzmake api --dayz dz enum --json EStatTypes | jq -c '.result.Enums[0].en.items[] | {name, value}'
{"name":"HEALTH","value":0}
{"name":"BLOOD","value":1}
{"name":"SHOCK","value":10}
{"name":"WATER","value":11}
$ dayzmake api --dayz dz src --json -m @MyMod PlayerBase | jq -r '.result.Sources[] | "\(.index) \(.pbo) \(.path):\(.line)"'
vanilla dta/scripts.pbo 4_world\player.c:1
_MyMod addons/mymod.pbo 4_world\mymod.c:1
```

The key inside `result` names the kind: `Classes`, `Matches`, `Names`, `Enums`, `Chain`, `Sources`, `Defines`.
`truncated` is the number of items that `--max` cut off. The text output of the same commands is the stable thing
to read for humans; the JSON is the stable thing to program against.

## The cache

Indexes are cached as JSON files, so later queries are instant. A cache is rebuilt by itself when a PBO changes
(size or modification time). `--refresh` rebuilds it by force, `api update` rebuilds without answering a query,
and deleting the folder is always safe. Where it lives: [Environment and files](../reference/environment.md). If
the folder cannot be written, queries still work, slowly, with the warning `api-cache`.

The same lookups are available to AI assistants as tools: see [Using dayzmake from an MCP client](mcp.md).
