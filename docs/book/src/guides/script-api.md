# Looking up the script API

DayZ scripts (Enforce Script) have no official reference that stays current. `dayzmake api` reads the scripts of
the installed game, `dta/scripts.pbo`. It indexes the classes, members and enums once. It then answers lookups in
milliseconds and does not extract anything. It can also index the scripts of mods. Then you see the vanilla
declarations and the mod declarations side by side.

The examples use a tiny stand-in game. This keeps the output short and reproducible. It has `Entity`, `ItemBase`,
`PlayerBase`, `Apple` and the enum `EStatTypes`. Use `--dayz dz` to select the stand-in. With your real Steam
install, leave out `--dayz`. dayzmake finds the game itself (see
[Environment and files](../reference/environment.md)).

## Who calls this?

`api find` matches only declarations. To find call sites and other usages, search the script bodies with
`api grep`. It matches the lines as they are in the source, with the comments:

```text
$ dayzmake api grep 'GetGame\(\)\.GetMission\(\)' --max 2
scripts\3_Game\Sandstorm.c:95: \t\t\tGetGame().GetMission().GetWorldData().WeatherSandstormStart(continuedSession);
scripts\3_Game\Sandstorm.c:115: \t\t\tGetGame().GetMission().GetWorldData().WeatherSandstormStop(calledByWeather);
... 4 more lines (raise --max or narrow the query)
```

For a case-insensitive match, add `--ignore-case`. To search the scripts of a mod too, add `-m cf`. See
[api grep](../reference/api.md#dayzmake-api-grep).

## Reading the game sources

If the index does not answer your question, extract the sources once. Then use your editor and `grep`. `extract`
unpacks each PBO of an install under its prefix and derapifies the configs. So `dz\scripts` becomes `dz/scripts`,
and each `config.bin` becomes a `config.cpp`:

```text
$ dayzmake extract --derapify --only '*.c' --only '*.bin' --only '*.cpp' --only '*.h' ~/DayZ game-sources
```

`game-sources/pbo-map.tsv` shows the PBO that each prefix came from. After a game update, run the command again with
`-f`. dayzmake replaces the old tree in one step, and the files that no longer exist are gone. A mod folder (`@Foo`)
or a single PBO works the same way. See [extract](../reference/pbo.md#dayzmake-extract).

## What does this class have?

```text
$ dayzmake api --dayz dz class PlayerBase
class PlayerBase : Entity
  bool IsAlive()  // Returns true while the player is alive.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
```

The first line shows the class and its base. dayzmake sorts the members. The documentation comments (`//!`) from the
source follow the member. `class` shows the class itself. To see the members of the base classes too, add
`--inherited`. To see more than 120 lines, add `--max`.

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

## What does this config class inherit?

`api config` is the same lookup for configs (`config.bin` and `config.cpp`). It answers questions such as: what
does `CfgVehicles/Hatchback_02` inherit, and what is its resolved `displayName`? It merges the configs of the game
and of the mods in load order, as the engine does. So the answer includes the changes that mods make.

```text
$ dayzmake api config CfgVehicles/Hatchback_02 --max 6
CfgVehicles/Hatchback_02
chain: Hatchback_02 : CarScript : Car : Transport : All
defined in (2):
  [vanilla] Addons/vehicles_wheeled.pbo
  [vanilla] Addons/sounds_hpp.pbo
properties (77):
... 80 more lines (raise --max or narrow the query)
```

Each property has a tag with the class it comes from (`displayName = "..."  (from Hatchback_02)`). The lines after
the properties list the classes declared inside the class and its subclasses. To see a class as the mods leave it,
add `-m <mods>`. For a structured object, add `--json`. The first run builds one cached index for each source. For
the game, this takes a short time. Later runs read the cache. See
[api config](../reference/api.md#dayzmake-api-config).

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

`src` shows the PBO, the path inside it and the line. `src --cat` prints the files, with a header line for each.
Use it to read the code around a declaration.

## Search

`find` takes a case-insensitive regular expression. It searches member names, signatures, doc comments, class names
and enum items. To search for names, anchor the expression:

```text
$ dayzmake api --dayz dz find '^OnRPC'
Entity: void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
PlayerBase: void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
$ dayzmake api --dayz dz find --class ItemBase quantity
ItemBase: int m_Quantity
ItemBase: int GetQuantity()  // Quantity of the item in its stack.
ItemBase: void SetQuantity(int value,bool destroy_config=true)
```

`enum` shows the values of an enum. It calculates them as the compiler does (an item without a value continues from
the previous item):

```text
$ dayzmake api --dayz dz enum EStatTypes
enum EStatTypes
  HEALTH = 0
  BLOOD = 1
  SHOCK = 10
  WATER = 11
```

## Adding mods

Use `-m` to select a mod. Without `-m`, dayzmake queries only vanilla. So a query never includes mods by surprise.

```text
$ dayzmake api --dayz dz class -m @MyMod PlayerBase
class PlayerBase : Entity  [vanilla 4_world]
  bool IsAlive()  // Returns true while the player is alive.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
class PlayerBase  [_MyMod 4_world]  [modded]
  void Heal(float amount)  // Added by MyMod.
```

The `modded class` of the mod adds `Heal`. `--mods-only` shows only the part of the mod. `-m` accepts:

- a folder, such as `@MyMod` above (it needs an `addons` folder with PBOs);
- a Steam Workshop id, or the name of an installed mod (`-m "my mod"`; a unique part of the name is enough);
- the aliases `cf`, `cot`, `dabs` and `expansion` for the known frameworks, or `aliases` for all four;
- `all` for every installed mod that has scripts.

`dayzmake api mods` lists what is installed:

```text
$ dayzmake api mods
1234567890  My Mod  …/steamapps/workshop/content/221100/1234567890  scripts
```

The first query with a mod indexes its PBOs. This can take a while for big mods. dayzmake caches the result, and a
query with `-m all` indexes each mod only once. To do this work before you need it, for example after a game
update, run:

```text
$ dayzmake api -m all update
vanilla-stable: 4 classes, stable build 17000000
mod-My_Mod-stable-ceb44f75: 2 classes, stable
```

`defines` lists the compile-time defines of mods (the `defines[]` of `CfgMods` and `#define` in scripts):

```text
$ dayzmake api --dayz dz defines -m @MyMod 'MYMOD|SERVER'
[_MyMod] mymod: MYMOD
```

## Branches

The default is the stable branch. `--experimental` queries the experimental branch. dayzmake searches the same
Steam libraries for its install. If the branch is not installed, you get an error that tells you to pass
`--dayz <dir>`.

## JSON for scripts and tools

Each query accepts `--json`. It then prints one line of JSON: `{"result": {...}, "truncated": N}`. Pipe it into
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
`truncated` is the number of items that `--max` cut off. Humans can rely on the text output of these commands. Programs
can rely on the JSON.

## The cache

dayzmake caches the indexes as JSON files, so later queries are instant. It rebuilds a cache by itself when a PBO
changes (size or modification time). `--refresh` forces a rebuild. `api update` rebuilds without a query. It is
always safe to delete the folder. For the location of the cache, see
[Environment and files](../reference/environment.md). If dayzmake cannot write to the folder, queries still work,
but slowly, and you get the warning `api-cache`.

AI assistants can use the same lookups as tools. See [Using dayzmake from an MCP client](mcp.md).
