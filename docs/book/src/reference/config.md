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

Behaviour:

- Reads `<source>`, runs the preprocessor (`#include`, `#define`, `#ifdef`, `__EVAL`, `__EXEC`, macros), parses
  the result as a config and writes it rapified.
- Relative includes (`#include "inc/common.hpp"`) are resolved against the folder of the file that contains
  the directive. Absolute includes (starting with `\`) are searched in the `-i` folders by path suffix and
  `$PBOPREFIX$`. A missing include is an error naming the file and line.
- A source that is already rapified is copied unchanged with the `prerapified-file` warning.
- Syntax errors name the line and the token that was expected. Values that are not quoted are accepted with the
  `unquoted-string` warning and an entry without a value with `empty-value`.
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
$ dayzmake rapify bad.cpp        # contains:  class A { x = 1
In file bad.cpp:1: warning: String value "1" is not quoted properly. [unquoted-string]
error: In line bad.cpp:2:

  
  ^

Unexpected token "\n", expected: one of ";", "__EXEC", "class", "delete", "}", ['a'..='z' | 'A'..='Z' | '0'..='9' | '_']
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
  a macro is called with the wrong number of arguments.

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
| `-d <indentation>`, `--indent <indentation>` | four spaces | Meant to set the indentation string. It is accepted but has no effect in 3.0.0: the output always uses four spaces. |

Behaviour:

- The input must be a rapified config; anything else is an error `Failed to read rapified config:`.
- Macros and includes are already expanded in a rapified config, so the text is the expanded result. Comments
  are not stored and cannot be restored.
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
