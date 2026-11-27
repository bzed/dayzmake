# Configs: rapify, preprocess, derapify

Config files (`config.cpp`, `.rvmat`, `model.cfg`) are text. DayZ reads them in a binary form called *rapified*.
These three commands convert between the two forms and run the preprocessor on its own. The examples use this layout
(see [Config files and the preprocessor](../guides/configs.md) for the files):

```text
cfg/config.cpp
cfg/inc/common.hpp
shared/$PBOPREFIX$
shared/lib/defs.hpp
```

## dayzmake rapify

Preprocess a config and write it in the binary format.

<div class="synopsis">

```text
dayzmake rapify [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
```

</div>

Arguments:

| Argument | Meaning |
|---|---|
| `<source>` | The config text. If you omit it, dayzmake reads standard input and resolves includes against the current directory. |
| `<target>` | The rapified file. If you omit it, dayzmake writes to standard output. |

Options:

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning instead of ten per name. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-w <wname>`, `--warning <wname>` | none | Mute a [warning](warnings.md) by name. Repeatable. |
| `-i <includefolder>`, `--include <includefolder>` | none | Folder that dayzmake searches for absolute includes (`#include "\x\y.hpp"`). Repeatable. For the current directory, dayzmake only checks the exact include path and does not search. |

dayzmake accepts an `enum { A, B = 5, C };` block at any class level. It is not a class entry. dayzmake puts its
names, in lowercase and in source order, into the enum table of the rapified file. Values start at 0 and count up
from the last explicit value, as in C. A name that occurs twice does not cause a warning, as in CfgConvert. The last
value counts.

dayzmake computes values in `f32`, as CfgConvert does. It rounds a literal to `f32`. A value outside `i32` is
`-2147483648` (so are `2147483647`, `2147483648`, `99999999999999999999` and `-2147483649`). Hex keeps its low 32
bits at any length (`0x1FFFFFFFFF` is `-1`) and `0X` works. An implicit value is the last explicit value plus its
distance, added in `f32`: `A = 16777217, B, C` gives `16777216`, `16777216`, `16777218`. `__EVAL` of large numbers
keeps its exact floats. dayzmake does not copy the imprecise low bits of CfgConvert.

Behaviour:

- It reads `<source>`, runs the preprocessor (`#include`, `#define`, `#ifdef`, `__EVAL`, `__EXEC`, macros), parses
  the result as a config and writes it rapified.
- It resolves relative includes (`#include "inc/common.hpp"`) against the folder of the file that contains
  the directive. It searches absolute includes (they start with `\`) in the `-i` folders by path suffix and by
  `$PBOPREFIX$`. A missing include is an error that names the file and the line.
- If the source is already rapified, dayzmake copies it unchanged and gives the `prerapified-file` warning.
- A syntax error names the line and the token that dayzmake expected. dayzmake drops the warnings that the failed
  parse raised on the line of the error. It keeps the warnings of earlier lines. It accepts values that are not
  quoted and gives the `unquoted-string` warning. It accepts an entry without a value and gives `empty-value`.
- A value entry (not an array or a class) can end at the line end instead of `;`, as in CfgConvert. `x = 1.5` keeps
  its number type. dayzmake evaluates `"s"` and `__EVAL(...)` values as it does with `;` (`1e999` is an infinity,
  see below). It handles an integer that overflows as it does with `;` (next bullet). An entry on the last line of a
  file that has no line end after it is a syntax error, because CfgConvert rejects it too.
- dayzmake stores a decimal integer by size, as CfgConvert does. An `i32` is a 32-bit integer. A number that fits 64
  bits but not 32 bits (`2147483648`) is a 64-bit integer entry (entry subtype 6; `derapify` and `api config` read
  it). A number that does not fit 64 bits (`99999999999999999999`) is a float (`1e20`). Arrays have no 64-bit
  element type. In an array, a decimal that does not fit 32 bits (`3000000000`) is a float. If text follows the
  value, the value stays an unquoted string.
- A hex literal without a sign keeps its low 32 bits at any length (`0x1FFFFFFFFF` is `-1`, `0x100000000` is `0`).
  `0X` works like `0x`. With a sign, the literal is a float (`-0x10` is `-16.0`).
- A float that does not fit an f32 (`1e999`, `-1e39`) becomes an infinity, as in CfgConvert. A float that underflows
  (`1e-999`) becomes 0. `derapify` writes an infinity as `1e999` or `-1e999`, so `derapify` followed by `rapify`
  keeps the value bit for bit. If text follows the value (`1e999x`), the value stays an unquoted string.
- dayzmake supports `delete`, `class X: Y`, forward declarations and `+=` array appends.

Examples:

```text
$ dayzmake rapify -i ../shared config.cpp config.bin
$ dayzmake derapify config.bin
class CfgVehicles {
    class Inventory_Base;
    class MyItem: Inventory_Base {
        scope = 2;
        displayName = "My item";
        itemSize[] = {4, 4};
        descriptionShort = "Spare part";
        weight = 100;
        absorbency = 0.5;
    };
};
```

The preprocessor does not evaluate a macro argument that is an array element:

```text
$ dayzmake rapify -i ../shared config.cpp -f config.bin      # with  {ITEM_SIZE, DOUBLE(2)}
In file config.cpp:10: warning: String array element "((2)*2)" is not quoted properly. [unquoted-string]
```

Use `__EVAL(2*2)` for arithmetic, as in the config above.

dayzmake protects an existing target:

```text
$ dayzmake rapify -i ../shared config.cpp config.bin
error: Target file "config.bin" already exists. Use --force to overwrite.
```

A syntax error:

```text
$ dayzmake rapify bad.cpp        # contains:  class A { x = 1   (no final newline)
error: In line bad.cpp:1:

  class A { x = 1
                 ^

Unexpected token "\n", expected: one of ".", ";", ['0'..='9'], ['e' | 'E'], [_]
```

See also: [derapify](#dayzmake-derapify), [preprocess](#dayzmake-preprocess), [Config files and the
preprocessor](../guides/configs.md), [Header extensions and prefixes](header-extensions.md).

## dayzmake preprocess

Run only the preprocessor and print the result.

<div class="synopsis">

```text
dayzmake preprocess [-v] [-f] [-w <wname>]... [-i <includefolder>]... [<source> [<target>]]
```

</div>

The arguments and options are those of [rapify](#dayzmake-rapify). `<source>` defaults to standard input and
`<target>` defaults to standard output. The options `-v`, `-f`, `-w` and `-i` have the same meaning.

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-w <wname>`, `--warning <wname>` | none | Mute a warning by name. |
| `-i <includefolder>`, `--include <includefolder>` | none | Folder searched for absolute includes. |

Behaviour:

- The output is the text after includes, macros and conditionals. dayzmake removes leading indentation, and the
  directives leave blank lines.
- This command does not evaluate `__EVAL`. `rapify` evaluates it when it parses the config.
- The `include-case` warning appears when an include matches only if you ignore case. The `macro-arg-count`
  warning appears when a macro has the wrong number of arguments. A `//` comment can end at the end of the file
  without a line end. An unterminated `/*` runs to the end of the file, as in CfgConvert, and gives the warning
  `unterminated-comment`.

Example:

```text
$ dayzmake preprocess -i ../shared config.cpp


class CfgVehicles
{
class Inventory_Base;
class MyItem: Inventory_Base
{
scope = 2;
displayName = "My item";
itemSize[] = {4, __EVAL(2*2)};
descriptionShort = "Spare part";
weight = 100;
absorbency = 0.5;
};
};

```

Without `-i`, dayzmake cannot find the absolute include:

```text
$ dayzmake preprocess config.cpp
error: File "\shared\lib\defs.hpp" included from "config.cpp:2" not found.
```

See also: [rapify](#dayzmake-rapify), [Config files and the preprocessor](../guides/configs.md).

## dayzmake derapify

Convert a rapified config back to text.

<div class="synopsis">

```text
dayzmake derapify [-v] [-f] [-d <indentation>] [<source> [<target>]]
```

</div>

| Argument | Meaning |
|---|---|
| `<source>` | The rapified config (a `config.bin`). If you omit it, dayzmake reads standard input. |
| `<target>` | The text file. If you omit it, dayzmake writes to standard output. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-d <indentation>`, `--indent <indentation>` | four spaces | The string that dayzmake writes once for each nesting level. Use only spaces and tabs, at most 64 characters. An empty string gives output without indentation. Anything else is an error, and dayzmake writes nothing. |

Behaviour:

- The input must be a rapified config. Anything else is the error `Failed to read rapified config:`.
- A rapified config has the macros and includes already expanded, so the text is the expanded result. The rapified
  file does not store comments, so dayzmake cannot restore them.
- dayzmake prints the enum table of the file first, as an `enum { name = value, ... };` block, with the names as
  stored (lowercase). A table that is empty, missing or damaged gives no block and no error.
- dayzmake limits the class nesting depth and the entry counts when it reads, so a hostile file cannot use all memory.

Examples:

```text
$ dayzmake derapify config.bin
class CfgVehicles {
    class Inventory_Base;
    class MyItem: Inventory_Base {
        scope = 2;
        displayName = "My item";
        itemSize[] = {4, 4};
        descriptionShort = "Spare part";
        weight = 100;
        absorbency = 0.5;
    };
};
```

`delete` and `+=` stay after the round trip:

```text
$ dayzmake derapify t.bin
class CfgFoo {
    class A {};
    class B: A {
        x = 1;
    };
    delete A;
    arr[] += {1};
};
```

See also: [rapify](#dayzmake-rapify), [unpack](pbo.md#dayzmake-unpack) (to get `config.bin` out of a PBO).
