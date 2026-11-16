# Script API: api

`dayzmake api` answers questions about the DayZ script API (classes, members, enums, base class chains, where a
declaration lives, compile-time defines) and about its configs (`api config`) from the installed game, without
extracting anything. It reads
`dta/scripts.pbo` of the game and, when asked with `-m`, the script PBOs of mods, builds an index once and
caches it. The [guide](../guides/script-api.md) shows typical sessions; this page lists every command.

The examples use a small synthetic game (`dz/dta/scripts.pbo` with the classes `Entity`, `ItemBase`, `PlayerBase`,
`Apple` and the enum `EStatTypes`) selected with `--dayz dz`, and a mod `@MyMod` that adds `Heal` to `PlayerBase`.
With a real installation `--dayz` is not needed.

## Selecting mods

`-m` (`--mod`) takes a comma separated list. Without it only vanilla is queried. Each entry is one of:

| Entry | Meaning |
|---|---|
| `cf`, `cot`, `dabs`, `expansion` | Community Framework, Community Online Tools, DabsFramework, DayZ Expansion, by their workshop ids; the mod must be subscribed on Steam. |
| `aliases` | All four aliases above. |
| `all` | Every installed mod that has scripts. The first run indexes each mod, which can take a while; the result is cached. |
| a number | A workshop id. |
| a directory | A mod folder that has an `addons` folder with PBOs. A name that exists as a path, or contains `/` or `\`, is taken as a directory. |
| a name | The `name` of an installed mod's `meta.cpp` or its `@folder` name, matched ignoring case; a unique part of the name is enough. See [api mods](#dayzmake-api-mods). |

Indexes are named `vanilla` and, for mods, the alias, the workshop id, or the mod's folder name or title with
characters other than letters, digits, `.`, `_` and `-` replaced by `_` (`@MyMod` becomes `_MyMod`). The text output tags lines of mods with the index name, for example `[My_Mod]`.

All the query commands below load vanilla first and then the selected mods, and answer over all of them.
An index whose input files changed (size or modification time of a PBO, or a new format of the index) is rebuilt
automatically; the cache location is in [Environment and files](environment.md).

## Text and JSON output

Text output has one declaration per line; members carry their `//!` doc comment after `//`. JSON output is one
line, `{"result": {...}, "truncated": N}`, where `N` is the number of items cut by `--max`. The key inside
`result` names the kind: `Classes`, `Matches`, `Names`, `Enums`, `Chain`, `Sources` or `Defines`. `src --cat --json`
returns `{"files": [...]}`, as do `--context` and `--lines` with `--json`. Declarations carry `module` (index into the PBO list), `path` and `line`.

## dayzmake api class

Show the members of a class.

<div class="synopsis">

```text
dayzmake api class [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--inherited] [--mods-only] [--json] <name>
```

</div>

| Argument | Meaning |
|---|---|
| `<name>` | The class name (exact, case-sensitive). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--inherited` | off | Also show the members of the base classes. |
| `--mods-only` | off | Show only what the selected mods declare. |
| `--max <n>` | 120 | Print at most `n` lines (JSON: items). A cut is announced with `... N more lines (raise --max or narrow the query)`. |
| `--json` | off | Print JSON instead of text. |


Behaviour: members are listed alphabetically with modifiers (`static`, `protected`, `native`), types, default
values and doc comments. `modded class` declarations of mods are shown as separate blocks tagged `[modded]`.
With `--inherited` the members of each base class follow, nearest first. `--mods-only` hides everything the
selected mods do not declare, and is an error if no selected mod declares the class. A class that does not exist
is an error that suggests `api find`.

Examples:

```text
$ dayzmake api --dayz dz class PlayerBase
class PlayerBase : Entity
  bool IsAlive()  // Returns true while the player is alive.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
```

```text
$ dayzmake api --dayz dz class -m @MyMod PlayerBase
class PlayerBase : Entity  [vanilla 4_world]
  bool IsAlive()  // Returns true while the player is alive.
  void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
class PlayerBase  [_MyMod 4_world]  [modded]
  void Heal(float amount)  // Added by MyMod.
```

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

```text
$ dayzmake api --dayz dz class Nope
error: no class "Nope"; try: dayzmake api find Nope
  (searched vanilla only; add -m <mod> or -m all for mods)
```

```text
$ dayzmake api class --max 1 PlayerBase
class PlayerBase : Entity
... 2 more lines (raise --max or narrow the query)
```

See also: [api find](#dayzmake-api-find), [api chain](#dayzmake-api-chain), [api src](#dayzmake-api-src).

## dayzmake api find

Search names, signatures and documentation with a regular expression.

<div class="synopsis">

```text
dayzmake api find [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--class <class>] [--json] <regex>
```

</div>

| Argument | Meaning |
|---|---|
| `<regex>` | A case-insensitive regular expression. It is matched against member names, signatures, doc comments, class names and enum items, so `^OnRPC` finds the `OnRPC` methods. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--class <class>` | all | Search only the members of this class or enum. |
| `--max <n>` | 120 | Print at most `n` lines (JSON: items). A cut is announced with `... N more lines (raise --max or narrow the query)`. |
| `--json` | off | Print JSON instead of text. |


Behaviour: every match is printed as `<Class>: <declaration>`; with mods, lines are prefixed with the index in
brackets. An invalid regular expression is an error.

Examples:

```text
$ dayzmake api --dayz dz find '^OnRPC'
Entity: void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
PlayerBase: void OnRPC(PlayerIdentity sender,int rpc_type,ParamsReadContext ctx)
```

```text
$ dayzmake api --dayz dz find -m @MyMod 'heal|health'
[vanilla] Entity: protected float m_Health
[vanilla] Entity: float GetHealth()  // Current health of the entity.
[vanilla] Entity: void SetHealth(float value)
[vanilla] EStatTypes: HEALTH = 0
[_MyMod] PlayerBase: void Heal(float amount)  // Added by MyMod.
```

```text
$ dayzmake api --dayz dz find --class ItemBase quantity
ItemBase: int m_Quantity
ItemBase: int GetQuantity()  // Quantity of the item in its stack.
ItemBase: void SetQuantity(int value,bool destroy_config=true)
```

See also: [api class](#dayzmake-api-class), [api enum](#dayzmake-api-enum).

## dayzmake api sub

List the subclasses of a class.

<div class="synopsis">

```text
dayzmake api sub [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--all] [--json] <name>
```

</div>

| Argument | Meaning |
|---|---|
| `<name>` | The class name. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--all` | off | Include indirect subclasses. |
| `--max <n>` | 120 | Print at most `n` lines (JSON: items). A cut is announced with `... N more lines (raise --max or narrow the query)`. |
| `--json` | off | Print JSON instead of text. |


Behaviour: prints the names on one line. Without `--all` only direct subclasses are listed.

Examples:

```text
$ dayzmake api --dayz dz sub Entity
ItemBase PlayerBase
$ dayzmake api --dayz dz sub --all Entity
ItemBase PlayerBase Apple
```

See also: [api chain](#dayzmake-api-chain).

## dayzmake api enum

Show the items of an enum.

<div class="synopsis">

```text
dayzmake api enum [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] <name>
```

</div>

| Argument | Meaning |
|---|---|
| `<name>` | The enum name. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--max <n>` | 120 | Print at most `n` lines (JSON: items). A cut is announced with `... N more lines (raise --max or narrow the query)`. |
| `--json` | off | Print JSON instead of text. |


Behaviour: the values are computed, so an item without an explicit value continues counting from the previous
one. Items added by `modded enum` declarations of mods are included, tagged with their index.

Example:

```text
$ dayzmake api --dayz dz enum EStatTypes
enum EStatTypes
  HEALTH = 0
  BLOOD = 1
  SHOCK = 10
  WATER = 11
```

See also: [api find](#dayzmake-api-find).

## dayzmake api chain

Show the chain of base classes of a class.

<div class="synopsis">

```text
dayzmake api chain [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--json] <name>
```

</div>

| Argument | Meaning |
|---|---|
| `<name>` | The class name. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--json` | off | Print JSON instead of text. |


Behaviour: the class, its base, the base of that, up to the root, joined with `>`. There is no `--max`; chains
are short.

Example:

```text
$ dayzmake api --dayz dz chain Apple
Apple > ItemBase > Entity
```

See also: [api sub](#dayzmake-api-sub), [api class](#dayzmake-api-class).

## dayzmake api src

Show where a class or enum is declared, or print the files.

<div class="synopsis">

```text
dayzmake api src [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--cat] [--context <n>] [--lines <a-b>] [--json] <name>
```

</div>

| Argument | Meaning |
|---|---|
| `<name>` | The class or enum name. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--cat` | off | Print the full source files instead of the locations. |
| `--context <n>` | off | Print the lines from `line-n` to `line+n` of each file that declares the name, with line numbers. |
| `--lines <a-b>` | off | Print lines `a` to `b` of each file that declares the name, with line numbers. |
| `--json` | off | Print JSON instead of text. |


Behaviour: without `--cat`, one line per declaration: `[<index>] <pbo> <prefix>\<path>:<line>`. With `--cat`, the
full source files are read from the PBOs again and printed, each after a `// ==== [<index>] <pbo> <path>:<line>`
header, with a blank line between files. `--context` and `--lines` print part of each file under the same header,
one `<line>| <text>` per line, the number right-aligned to the width of the largest number in the range (`  9| ...`,
` 10| ...`); a range outside the file is clamped to it, `a` greater than `b` is an error, and the two options exclude
each other and `--cat`. With `--json` each file carries `"lines": [{"line": n, "text": "..."}]` instead of `text`. `--max` does not apply. In the MCP server `api_source` can write the files to a file.

Examples:

```text
$ dayzmake api --dayz dz src -m @MyMod PlayerBase
[vanilla] dta/scripts.pbo scripts\4_world\player.c:1
[_MyMod] addons/mymod.pbo mymod\4_world\mymod.c:1
```

```text
$ dayzmake api --dayz dz src --cat Apple
// ==== [vanilla] dta/scripts.pbo scripts\4_world\player.c:8
class PlayerBase extends Entity
{
	//! Returns true while the player is alive.
	bool IsAlive();
	void OnRPC(PlayerIdentity sender, int rpc_type, ParamsReadContext ctx);
}

class Apple extends ItemBase
{
	void Eat(PlayerBase player);
}
```

See also: [api class](#dayzmake-api-class).

## dayzmake api grep

Search the script bodies for a regular expression.

<div class="synopsis">

```text
dayzmake api grep [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--max <n>] [--ignore-case] [--json] <regex>
```

</div>

| Argument | Meaning |
|---|---|
| `<regex>` | A regular expression, as `api find` takes it, matched against each line. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Accepted for consistency; `grep` prints no per-index header. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the scripts are read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also search mods; see [Selecting mods](#selecting-mods). |
| `--max <n>` | 120 | Print at most `n` lines (JSON: items). The rest is counted and announced with `... N more lines (raise --max or narrow the query)`. |
| `--ignore-case` | off | Match case-insensitively. Without it the match is case-sensitive, unlike `api find`. |
| `--json` | off | Print a JSON array of `{"source", "path", "line", "text"}` instead of text. |

Behaviour: every `.c` entry of `dta/scripts.pbo` and of the script PBOs of the selected mods is read straight
from its PBO and matched line by line on the raw source, comments included; nothing is written to disk and no
index is needed (`--refresh` does not apply). Output is `<prefix>\<path>:<line>: <text>`, the path as the
`<prefix>\<path>` part of `api src` output (without the PBO file), with `[<index>] ` in front when mods are selected. Hits are sorted by source (vanilla first, then the mods
in `-m` order), then path, then line. `--max` limits the printed lines but the whole input is scanned, so the
number in the `... N more lines` note is exact (a full scan of the game's scripts takes well under a second).
`no match` is printed when nothing matches; control characters in the text are escaped (a tab shows as `\t`).
JSON has no truncation note: it holds the first `--max` hits. An invalid regex is an error that names the pattern.
A mod PBO that cannot be read is skipped with a warning.

Examples:

```text
$ dayzmake api grep 'GetGame\(\)\.GetMission\(\)' --max 2
scripts\3_Game\Sandstorm.c:95: \t\t\tGetGame().GetMission().GetWorldData().WeatherSandstormStart(continuedSession);
scripts\3_Game\Sandstorm.c:115: \t\t\tGetGame().GetMission().GetWorldData().WeatherSandstormStop(calledByWeather);
... 4 more lines (raise --max or narrow the query)
```

See also: [api find](#dayzmake-api-find), [api src](#dayzmake-api-src).

## dayzmake api defines

List the compile-time defines that are active in the selected indexes.

<div class="synopsis">

```text
dayzmake api defines [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] [<regex>]
```

</div>

| Argument | Meaning |
|---|---|
| `<regex>` | Optional filter; only defines that match are printed. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# <index>: <build>` line before the result of each index. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory; the index is read from `<dir>/dta/scripts.pbo`. |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also query mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached index before answering. |
| `--max <n>` | 120 | Print at most `n` lines (JSON: items). A cut is announced with `... N more lines (raise --max or narrow the query)`. |
| `--json` | off | Print JSON instead of text. |


Behaviour: lists the defines that the configs and scripts of each index declare (`#define` in scripts and the
`defines[]` of `CfgMods` in a mod's config), per PBO prefix. Output is `[<index>] <prefix>: <name> <name> ...`;
when nothing matches it prints `no matching define`. The conditional compilation of the scripts (`#ifdef SERVER`
and so on) is evaluated with the defines of a dedicated server plus those of the mod when the index is built.

Examples:

```text
$ dayzmake api --dayz dz defines -m @MyMod 'MYMOD|SERVER'
[_MyMod] mymod: MYMOD
$ dayzmake api --dayz dz defines
no matching define
```

See also: [api class](#dayzmake-api-class).

## dayzmake api config

Look up a config class: its inheritance chain, its own and inherited properties, its subclasses and the PBOs that
define it.

<div class="synopsis">

```text
dayzmake api config [-v] [--experimental] [--dayz <dir>] [-m <mods>] [--refresh] [--max <n>] [--json] <path>
```

</div>

| Argument | Meaning |
|---|---|
| `<path>` | A `/`-separated class path such as `CfgVehicles/Hatchback_02`, matched ignoring case. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Print a `# config <source>: <n> classes` line for vanilla and each mod. |
| `--experimental` | off | Use the experimental branch instead of stable. |
| `--dayz <dir>` | Steam install | A DayZ client or server directory (it must have `dta/scripts.pbo`). |
| `-m <mods>`, `--mod <mods>` | vanilla only | Also merge the configs of mods; see [Selecting mods](#selecting-mods). |
| `--refresh` | off | Rebuild the cached config indexes before answering. |
| `--max <n>` | 120 | Print at most n lines; with `--json` at most n properties, n subclasses and n nested classes. |
| `--json` | off | Print one JSON object instead of text. |

Which configs: every rapified `config.bin` and every plain `config.cpp` of the PBOs in the `dta` and `addons`
folders of the game (any case), including the ones in folders of a PBO (the weapon PBOs keep one per weapon), and
of the `addons` folder of each selected mod. Everything else in the PBOs is left unread.

Load order and merging, as the engine does it:

- Vanilla loads first, then the mods in `-m` order. Inside the game, and inside each mod, the PBOs are sorted
  by `CfgPatches.*.requiredAddons` (an addon after the PBO that provides the addons it requires); sorted path order
  breaks ties. A cycle in `requiredAddons` is a warning (`api-config-order`) and the PBOs involved load in path order.
- A class defined again extends the earlier definition: its properties are added or overwritten and it keeps its
  base class unless the new definition names one. `delete X;` removes the class with everything inside it.
  `x[] += {...};` appends to the array of the earlier definition or, when the class has none, of the base class.
  A forward declaration (`class X;`) creates nothing.

Inheritance: for `class B: A` the base `A` is looked up in the scope that holds `B` first (never `B` itself), then
in the classes that scope inherits from, then in the scopes around it, up to the top level. That is how
`class HitPoints: HitPoints` inside a derived class finds the `HitPoints` of its base. A base that is defined
nowhere ends the chain with `(not found)`; a base that leads back to the class itself is an error
(`inheritance cycle`).

Output: the class path, the chain (`Hatch : Base`), the PBOs that define or modify the class (`[<source>] <pbo>`,
in load order), the resolved properties sorted by name, each tagged `(from <class>)` with the nearest class that
sets it (the last one that extended it, for `+=`), the classes declared inside the class (`classes`) and its direct
subclasses (classes whose base resolves to it). Long values are cut in text output. JSON is
`{"path", "name", "chain": [{"name", "path"}], "chain_end": "root" or "unknown base <name>",
"properties": [{"name", "value", "origin", "origin_path"}], "subclasses", "classes", "pbos": [{"source", "pbo"}],
"truncated": N}`.

The index is cached like the script index: one file per source (`config-vanilla-<branch>.json`,
`config-mod-<name>-<branch>-<hash>.json`) that is rebuilt when a PBO changes, with `--refresh`, or after an update of
dayzmake; the sources are merged on each call. A PBO or config that cannot be read is skipped with a warning
(`api-bad-config`); a source over the size limit (2 million classes or 10 million properties) is cut with
`api-config-limit`. `api update` does not build it.

Example:

```text
$ dayzmake api --dayz dz config CfgVehicles/Hatch
CfgVehicles/Hatch
chain: Hatch : Base
defined in (2):
  [vanilla] Addons/zz_a.pbo
  [vanilla] Addons/aa_b.pbo
properties (4):
  displayName = "Hatch B"  (from Hatch)
  extra = 5  (from Hatch)
  scope = 2  (from Hatch)
  tags = {"a", "b", "b_tag"}  (from Hatch)
classes (0):
subclasses (1):
  Newcar
$ dayzmake api --dayz dz config CfgVehicles/Doomed
error: no config class "CfgVehicles/Doomed"; the deepest class found is "CfgVehicles"
```

Here `aa_b.pbo` requires the addon of `zz_a.pbo`, so it loads after it although it sorts before. `Doomed` is
deleted by the second addon.

See also: [api class](#dayzmake-api-class), [api mods](#dayzmake-api-mods).

## dayzmake api update

Rebuild the cached indexes now.

<div class="synopsis">

```text
dayzmake api update [-v] [--experimental] [--dayz <dir>] [-m <mods>]
```

</div>

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `--experimental` | off | Use the experimental branch. |
| `--dayz <dir>` | Steam install | The DayZ directory. |
| `-m <mods>`, `--mod <mods>` | vanilla only | The mods to index; `-m all` warms the cache for every installed mod. |

Behaviour: always rebuilds, never answers a query. One line per index: its cache name, the number of classes and
the build. Use it after a game update when the first query would otherwise be slow.

Examples:

```text
$ dayzmake api update
vanilla-stable: 4 classes, stable build 17000000
$ dayzmake api -m all update
vanilla-stable: 4 classes, stable build 17000000
mod-My_Mod-stable-ceb44f75: 2 classes, stable
```

(`-m` is accepted before or after the command word.)

See also: [Environment and files](environment.md) for the cache.

## dayzmake api mods

List the installed mods.

<div class="synopsis">

```text
dayzmake api mods [-v] [--experimental] [--dayz <dir>] [--json]
```

</div>

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `--experimental` | off | Look in the experimental branch's workshop folder. |
| `--dayz <dir>` | Steam install | The DayZ directory whose `@mod` folders are included. |
| `--json` | off | Print JSON: `{"mods":[{"dir","has_scripts","id","link","name"}]}`. |

Behaviour: lists the mods in the workshop content folder of the branch plus every `@*` folder of the game
install, sorted by name. Text lines are `<workshop id or ->  <name>  <folder>  [scripts]`; `scripts` appears when
a PBO of the mod contains `.c` files. Only folder listings, `meta.cpp` and the PBO headers are read. The names
and ids can be used with `-m`.

Examples:

```text
$ dayzmake api mods
1234567890  My Mod  …/steamapps/workshop/content/221100/1234567890  scripts
$ dayzmake api mods --json
{"mods":[{"dir":"…/steamapps/workshop/content/221100/1234567890","has_scripts":true,"id":1234567890,"link":null,"name":"My Mod"}]}
```

Errors name what is missing:

```text
$ dayzmake api -m nonexist class X
error: no installed mod is named "nonexist" (installed: My Mod); see `dayzmake api mods`
$ dayzmake api -m cf class X
error: mod cf (workshop 1559212036) is not installed for the stable branch; subscribe to it on the Steam workshop or pass its directory
$ dayzmake api --experimental class X
error: no DayZ experimental install found in the Steam libraries; pass the install directory with --dayz <dir>
$ dayzmake api class --max abc PlayerBase
error: --max must be a number, not "abc"
```
