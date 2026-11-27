# Script API: api

`dayzmake api` answers questions about the DayZ script API from the installed game. It does not extract anything.
It shows classes, members, enums, base class chains, the place of a declaration and compile-time defines. It also
answers questions about the configs of the game (`api config`).

The command reads `dta/scripts.pbo` of the game. When you use `-m`, it also reads the script PBOs of mods. It builds
an index once and caches it. The [guide](../guides/script-api.md) shows typical sessions. This page lists every
command.

The examples use a small synthetic game. It is `dz/dta/scripts.pbo` with the classes `Entity`, `ItemBase`,
`PlayerBase`, `Apple` and the enum `EStatTypes`, selected with `--dayz dz`. The examples also use a mod `@MyMod`
that adds `Heal` to `PlayerBase`. With a real installation, you do not need `--dayz`.

## Selecting mods

`-m` (`--mod`) takes a list that is separated by commas. Without it, the command queries only vanilla. Each entry
is one of these:

| Entry | Meaning |
|---|---|
| `cf`, `cot`, `dabs`, `expansion` | Community Framework, Community Online Tools, DabsFramework, DayZ Expansion, by their workshop ids. You must subscribe to the mod on Steam. |
| `aliases` | All four aliases above. |
| `all` | Every installed mod that has scripts (`api config`: also a mod that has only a config, such as a retexture). The first run indexes each mod and can take a long time. The command caches the result. |
| a number | A workshop id. |
| a directory | A mod folder that has an `addons` folder with PBOs. The command treats a name as a directory when the name exists as a path or contains `/` or `\`. |
| a name | The `name` in the `meta.cpp` of an installed mod, or its `@folder` name. The match ignores case. A unique part of the name is enough. See [api mods](#dayzmake-api-mods). |

The index for vanilla is named `vanilla`. The index for a mod is named with the alias, the workshop id, or the
folder name or title of the mod. In that name, each character other than a letter, a digit, `.`, `_` and `-` becomes
`_` (`@MyMod` becomes `_MyMod`). The text output tags the lines of mods with the index name, for example `[My_Mod]`.

All the query commands below load vanilla first and then the selected mods. They answer over all of them.
The command rebuilds an index automatically when its input files change. This is a change of the size or the
modification time of a PBO, or a new format of the index. The cache location is in
[Environment and files](environment.md).

## Text and JSON output

Text output has one declaration on each line. Members have their `//!` doc comment after `//`. JSON output is one
line, `{"result": {...}, "truncated": N}`. `N` is the number of items that `--max` cut. The key inside `result` names
the kind: `Classes`, `Matches`, `Names`, `Enums`, `Chain`, `Sources` or `Defines`. `src --cat --json` returns
`{"files": [...]}`, and so do `--context` and `--lines` with `--json`. Declarations have `module` (an index into the
PBO list), `path` and `line`.

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


Behaviour: the command lists the members in alphabetical order. It shows modifiers (`static`, `protected`,
`native`), types, default values and doc comments. It shows the `modded class` declarations of mods as separate
blocks with the tag `[modded]`. With `--inherited`, the members of each base class follow, nearest first.
`--mods-only` hides everything that the selected mods do not declare. It gives an error if no selected mod declares
the class. A class that does not exist gives an error that suggests `api find`.

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


Behaviour: the command prints every match as `<Class>: <declaration>`. When you select mods, each line starts with
the index in brackets. An invalid regular expression gives an error.

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


Behaviour: the command prints the names on one line. Without `--all`, it lists only direct subclasses.

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


Behaviour: the command computes the values. An item without an explicit value continues the count from the
previous item. The result includes the items that the `modded enum` declarations of mods add. Each of these items
has the tag of its index.

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


Behaviour: the command prints the class, its base, the base of that base, and so on up to the root, joined with
`>`. There is no `--max` because chains are short.

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


Behaviour: without `--cat`, the command prints one line for each declaration:
`[<index>] <pbo> <prefix>\<path>:<line>`. With `--cat`, the command reads the full source files from the PBOs again
and prints them. Each file follows a `// ==== [<index>] <pbo> <path>:<line>` header, and a blank line separates the
files.

`--context` and `--lines` print a part of each file under the same header. They print one `<line>| <text>` for each
line. The number is right-aligned to the width of the largest number in the range (`  9| ...`, ` 10| ...`). The
command clamps a range outside the file to the file. If `a` is greater than `b`, the command gives an error. The
options `--context`, `--lines` and `--cat` exclude each other. With `--json`, each file has
`"lines": [{"line": n, "text": "..."}]` instead of `text`. `--max` does not apply. In the MCP server, `api_source`
can write the files to a file.

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
| `--json` | off | Print a JSON array of `{"source", "path", "line", "text"}` (plus `"truncated"` on cut lines) instead of text. |

Behaviour: the command reads every `.c` entry of `dta/scripts.pbo` and of the script PBOs of the selected mods
straight from its PBO. It matches the raw source line by line, comments included. It writes nothing to disk, and it
does not need an index (`--refresh` does not apply). The output is `<prefix>\<path>:<line>: <text>`. The path is the
`<prefix>\<path>` part of the `api src` output (without the PBO file). When you select mods, `[<index>] ` is in
front of the path.

- The command sorts the hits by source (vanilla first, then the mods in `-m` order), then by path, then by line.
- `--max` limits the printed lines, but the command scans the whole input. The number in the `... N more lines`
  note is therefore exact. A full scan of the game scripts takes less than a second.
- When nothing matches, the command prints `no match`.
- The command escapes control characters in the text (a tab shows as `\t`).
- The command prints a matching line up to its first 512 bytes (counted before escaping). It cuts the line back to
  the previous character boundary and adds ` […]`.
- In JSON, `text` is cut in the same way (without the marker). The hit gets `"truncated": true` only when the
  command cut it.
- JSON has no truncation note. It holds the first `--max` hits.
- An invalid regex gives an error that names the pattern.
- The command skips a mod PBO that it cannot read, and gives a warning.

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


Behaviour: the command lists the defines that the configs and scripts of each index declare, for each PBO prefix.
These are `#define` in scripts and the `defines[]` of `CfgMods` in the config of a mod. The output is
`[<index>] <prefix>: <name> <name> ...`. When nothing matches, the command prints `no matching define`. When it
builds the index, it evaluates the conditional compilation of the scripts (`#ifdef SERVER` and so on). It uses the
defines of a dedicated server and the defines of the mod.

Examples:

```text
$ dayzmake api --dayz dz defines -m @MyMod 'MYMOD|SERVER'
[_MyMod] mymod: MYMOD
$ dayzmake api --dayz dz defines
no matching define
```

See also: [api class](#dayzmake-api-class).

## dayzmake api config

Look up a config class. The result shows its inheritance chain, its own and inherited properties, its subclasses and
the PBOs that define it.

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

Which configs: the command reads every rapified `config.bin` and every plain `config.cpp` of the PBOs in the `dta`
and `addons` folders of the game (any case). This includes the files in folders of a PBO (the weapon PBOs keep one
for each weapon). It also reads the `addons` folder of each selected mod. `-m all` selects the installed mods that
have scripts or a config, so it includes retexture and tweak mods that have only a config. The command does not read
anything else in the PBOs.

Load order and merging, as the engine does them:

- The command sorts the config PBOs of the game and of all selected mods together by `CfgPatches.*.requiredAddons`.
  An addon comes after the PBO that provides the addons it requires. This is what the engine does. A game addon that
  requires a mod addon loads after it. A mod addon that a later mod requires loads first. The command breaks ties
  by the source (the game first, then the mods in `-m` order) and then by the sorted path. So, without dependencies,
  the order is the game, then the mods as you selected them. A cycle in `requiredAddons` gives a warning
  (`api-config-order`). The PBOs in the cycle load in the tie-break order.
- A class that is defined again extends the earlier definition. Its properties are added or overwritten. Like the
  engine, the command makes the base class the class that the new definition names. It makes no base class when the
  new definition names none. A mod that writes `class Hatchback_02 { ... }` without `: CarScript` cuts the class off
  from its base (the game logs `Updating base class`). The lookup shows this as
  `Base changed: CarScript -> (none) by [<source>] <pbo>`. JSON has `base_changes`. A change by a mod gives the
  warning `api-config-base-changed`. The command only shows the changes of the game itself.
- `delete X;` removes the class with everything inside it.
- `x[] += {...};` appends to the array of the earlier definition. When the class has no such array, it appends to
  the array of the base class.
- A forward declaration (`class X;`) creates nothing and changes nothing.

Inheritance: for `class B: A`, the command looks for the base `A` in these places, in this sequence. First, the
scope that holds `B` (never `B` itself). Then the classes that this scope inherits from. Then the scopes around it,
up to the top level. This is how `class HitPoints: HitPoints` inside a derived class finds the `HitPoints` of its
base. If no scope defines the base, the chain ends with `(not found)`. A base that leads back to the class itself
gives an error (`inheritance cycle`).

Output: the command prints these items.

- The class path.
- The chain (`Hatch : Base`).
- The PBOs that define or change the class (`[<source>] <pbo>`, in load order).
- The resolved properties, sorted by name. Each has the tag `(from <class>)` with the nearest class that sets it
  (for `+=`, the last class that extended it).
- The classes that are declared inside the class (`classes`).
- The direct subclasses (the classes whose base resolves to this class).

The text output cuts long values. A float that overflows (`1e999`) is an infinity. The text output shows it as
derapify writes it, `1e999` or `-1e999`. JSON has no infinity, so it shows the value as the string `"1e999"` or
`"-1e999"`. JSON is
`{"path", "name", "chain": [{"name", "path"}], "chain_end": "root" or "unknown base <name>",
"properties": [{"name", "value", "origin", "origin_path"}], "subclasses", "classes", "pbos": [{"source", "pbo"}],
"base_changes": [{"from", "to", "source", "pbo"}], "truncated": N}`.

The command caches the index like the script index. There is one file for each source
(`config-vanilla-<branch>.json`, `config-mod-<name>-<branch>-<hash>.json`). The command rebuilds the file when a PBO
changes, with `--refresh`, or after an update of dayzmake. It merges the sources on each call. The command skips a
PBO or config that it cannot read, and gives a warning (`api-bad-config`). It cuts a source that is over the size
limit (2 million classes or 10 million properties) and gives the warning `api-config-limit`. `api update` does not
build this index.

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

Here `aa_b.pbo` requires the addon of `zz_a.pbo`, so it loads after it, although it sorts before. The second addon
deletes `Doomed`.

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

Behaviour: the command always rebuilds the indexes and never answers a query. It prints one line for each index:
the cache name, the number of classes and the build. Use it after a game update. Then the first query is not slow.

Examples:

```text
$ dayzmake api update
vanilla-stable: 4 classes, stable build 17000000
$ dayzmake api -m all update
vanilla-stable: 4 classes, stable build 17000000
mod-My_Mod-stable-ceb44f75: 2 classes, stable
```

The command accepts `-m` before or after the command word.

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

Behaviour: the command lists the mods in the workshop content folder of the branch and every `@*` folder of the
game install, sorted by name. Text lines are `<workshop id or ->  <name>  <folder>  [scripts]`. `scripts` shows
when a PBO of the mod contains `.c` files. The command reads only folder listings, `meta.cpp` and the PBO headers.
You can use the names and ids with `-m`.

Examples:

```text
$ dayzmake api mods
1234567890  My Mod  …/steamapps/workshop/content/221100/1234567890  scripts
$ dayzmake api mods --json
{"mods":[{"dir":"…/steamapps/workshop/content/221100/1234567890","has_scripts":true,"id":1234567890,"link":null,"name":"My Mod"}]}
```

The errors name what is missing:

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
