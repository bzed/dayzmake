# Config files and the preprocessor

DayZ configs (`config.cpp`, `.rvmat`, `model.cfg`) are written in a C-like language with a preprocessor. This
guide shows what the preprocessor does, how includes are found and how to convert between the text and binary
forms. The example has this layout:

```text
cfg/config.cpp
cfg/inc/common.hpp
shared/$PBOPREFIX$              # contains one line: shared
shared/lib/defs.hpp
```

`cfg/inc/common.hpp`:

```cpp
#define ITEM_SIZE 4
#define LABEL(x) #x
```

`shared/lib/defs.hpp`:

```cpp
#define BASE_WEIGHT 100
```

`cfg/config.cpp`:

```cpp
#include "inc/common.hpp"
#include "\shared\lib\defs.hpp"
class CfgVehicles
{
    class Inventory_Base;
    class MyItem: Inventory_Base
    {
        scope = 2;
        displayName = "My item";
        itemSize[] = {ITEM_SIZE, __EVAL(2*2)};
        descriptionShort = LABEL(Spare part);
        weight = BASE_WEIGHT;
        absorbency = 0.5;
    };
};
```

## Two kinds of include

- **Relative includes** (`"inc/common.hpp"`) are looked up next to the file that contains the `#include`.
  Forward and backward slashes both work.
- **Absolute includes** (`"\shared\lib\defs.hpp"`, starting with a backslash) name a path inside a PBO. The folders
  given with `-i` are searched for a file whose path ends in that path, taking `$PBOPREFIX$` into account: the
  folder `shared` has the prefix `shared`, so `\shared\lib\defs.hpp` is `shared/lib/defs.hpp`. The current
  directory is only checked for the exact path, never searched, so it is safe to run dayzmake in a big folder.

Without `-i`, the absolute include is not found, and the error names file and line:

```text
$ cd cfg
$ dayzmake preprocess config.cpp
error: File "\shared\lib\defs.hpp" included from "config.cpp:2" not found.
```

With `-i`:

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

`ITEM_SIZE` and `BASE_WEIGHT` were replaced, `LABEL(Spare part)` became a quoted string through `#x`, and the
two include lines left blank lines. `__EVAL(2*2)` is only evaluated when the config is parsed.

If an include matches only when case is ignored, dayzmake uses it and warns (`include-case`), because the game
treats those names case-insensitively but your Linux file system does not:

```text
$ dayzmake rapify inc.cpp /dev/null -f
warning: Include "Inc/Common.HPP" only matched "…/inc/common.hpp" ignoring case. [include-case]
```

## Rapify and derapify

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

`__EVAL` has been calculated, the macros are gone and comments are not kept. `derapify` is how you read the
`config.bin` of any PBO (`dayzmake cat some.pbo config.bin | dayzmake derapify`). Both commands read standard input
and write standard output when the file arguments are left out, so they work in pipelines.

You rarely need `rapify` yourself: `build` rapifies every `.cpp` and `.rvmat` in the folder and passes the
include folders on (`dayzmake build -i ../shared mymod mymod.pbo`).

## Mistakes the parser reports

An unquoted value is accepted with a warning, and `delete` and `+=` are supported:

```text
$ dayzmake rapify warn.cpp w.bin        # class A { name = hello; z = ; };
In file warn.cpp:1: warning: String value "hello" is not quoted properly. [unquoted-string]
In file warn.cpp:1: warning: Entry "z" has no value; it is stored as an empty string. [empty-value]
$ dayzmake rapify -w unquoted-string -w empty-value warn.cpp -f w.bin
```

A real syntax error stops the command:

```text
$ dayzmake rapify bad.cpp        # class A { x = 1   (no final newline)
error: In line bad.cpp:1:

  class A { x = 1
                 ^

Unexpected token "\n", expected: one of ".", ";", ['0'..='9'], ['e' | 'E'], [_]
```

A macro called with the wrong number of arguments is a warning that names the line:

```text
$ dayzmake rapify mac.cpp -f /dev/null        # #define M(a,b) a+b / class A { x = M(1); };
In file mac.cpp:2: warning: Macro "M" expects 2 argument(s), got 1 [macro-arg-count]
In file mac.cpp:2: warning: Entry "x" has no value; it is stored as an empty string. [empty-value]
```

See the [reference](../reference/config.md) for every option.
