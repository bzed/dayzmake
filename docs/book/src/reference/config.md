# Configs: rapify, preprocess, derapify

Config files (`config.cpp`, `.rvmat`, `model.cfg`) are text that DayZ reads in a binary form called *rapified*.
These three commands convert between the two and run the preprocessor on its own. The examples use this layout
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
| `<source>` | The config text. Standard input when omitted; includes are then resolved against the current directory. |
| `<target>` | The rapified file. Standard output when omitted. |

Options:

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning instead of ten per name. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-w <wname>`, `--warning <wname>` | none | Mute a [warning](warnings.md) by name. Repeatable. |
| `-i <includefolder>`, `--include <includefolder>` | none | Folder searched for absolute includes (`#include "\x\y.hpp"`). Repeatable. The current directory is only checked for the exact include path, never searched. |

An `enum { A, B = 5, C };` block is accepted at any class level. It is no class entry: its names go, lowercase and
in source order, into the enum table of the rapified file. Values start at 0 and count up from the last explicit
one, as in C. A name that occurs twice is accepted without a warning, like CfgConvert; the last value counts.

Behaviour:

- Reads `<source>`, runs the preprocessor (`#include`, `#define`, `#ifdef`, `__EVAL`, `__EXEC`, macros), parses
  the result as a config and writes it rapified.
- Relative includes (`#include "inc/common.hpp"`) are resolved against the folder of the file that contains
  the directive. Absolute includes (starting with `\`) are searched in the `-i` folders by path suffix and
  `$PBOPREFIX$`. A missing include is an error naming the file and line.
- A source that is already rapified is copied unchanged with the `prerapified-file` warning.
- Syntax errors name the line and the token that was expected. Warnings the failed parse raised on the error's own line are dropped, those of earlier lines are kept. Values that are not quoted are accepted with the
  `unquoted-string` warning and an entry without a value with `empty-value`.
- A value entry (not an array or a class) may end at the line end instead of `;`, as in CfgConvert: `x = 1.5` keeps its number type, `"s"` and
  `__EVAL(...)` values are evaluated as with `;` (`1e999` is an infinity, see below), and an integer that overflows is handled as with `;` (next bullet). An entry on the last line of a file
  that has no line end after it is a syntax error, because CfgConvert rejects it too.
- A decimal integer that does not fit 64 bits (`99999999999999999999`) is a float (`1e20`), as in CfgConvert; in an array a decimal that does not fit 32 bits (`3000000000`) is a float. In an entry a literal that fits 64 but not 32 bits (`2147483648`) and a hex literal that overflows are errors (CfgConvert writes the first as a 64-bit integer, which dayzmake does not). Followed by text the value stays an unquoted string.
- A float that does not fit an f32 (`1e999`, `-1e39`) becomes an infinity, as in CfgConvert; one that underflows
  (`1e-999`) becomes 0. `derapify` writes an infinity as `1e999` or `-1e999`, so `derapify` followed by `rapify` keeps
  the value bit for bit. Followed by text (`1e999x`) the value stays an unquoted string.
- `delete`, `class X: Y`, forward declarations and `+=` array appends are supported.

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

A macro argument that is used as an array element is not evaluated by the preprocessor:

```text
$ dayzmake rapify -i ../shared config.cpp -f config.bin      # with  {ITEM_SIZE, DOUBLE(2)}
In file config.cpp:10: warning: String array element "((2)*2)" is not quoted properly. [unquoted-string]
```

Use `__EVAL(2*2)` for arithmetic, as in the config above.

Existing targets are protected:

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

Arguments and options are those of [rapify](#dayzmake-rapify): `<source>` defaults to standard input, `<target>`
to standard output, and `-v`, `-f`, `-w` and `-i` mean the same.

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Show every warning. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-w <wname>`, `--warning <wname>` | none | Mute a warning by name. |
| `-i <includefolder>`, `--include <includefolder>` | none | Folder searched for absolute includes. |

Behaviour:

- The output is the text after includes, macros and conditionals. Leading indentation is removed and the
  directives leave blank lines behind.
- `__EVAL` is not evaluated here; it is evaluated when the config is parsed by `rapify`.
- The `include-case` warning is raised when an include only matches ignoring case, and `macro-arg-count` when
  a macro is called with the wrong number of arguments. A `//` comment may end at the end of the file without a line
  end, and an unterminated `/*` runs to the end of the file, as in CfgConvert, with the warning `unterminated-comment`.

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

Without `-i`, the absolute include cannot be found:

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
| `<source>` | The rapified config (a `config.bin`). Standard input when omitted. |
| `<target>` | The text file. Standard output when omitted. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing `<target>`. |
| `-d <indentation>`, `--indent <indentation>` | four spaces | The string written once per nesting level. Only spaces and tabs are accepted, at most 64 characters; an empty string gives unindented output. Anything else is an error and nothing is written. |

Behaviour:

- The input must be a rapified config; anything else is an error `Failed to read rapified config:`.
- Macros and includes are already expanded in a rapified config, so the text is the expanded result. Comments
  are not stored and cannot be restored.
- The enum table of the file is printed first, as an `enum { name = value, ... };` block, with the names as stored
  (lowercase). A table that is empty, missing or damaged produces no block and no error.
- Class nesting depth and entry counts are limited when reading, so a hostile file cannot exhaust memory.

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

`delete` and `+=` survive the round trip:

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
