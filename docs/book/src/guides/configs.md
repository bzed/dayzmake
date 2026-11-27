# Config files and the preprocessor

DayZ configs (`config.cpp`, `.rvmat`, `model.cfg`) use a C-like language with a preprocessor. This guide shows what
the preprocessor does and how dayzmake finds includes. It also shows how to convert between the text form and the
binary form. The example has this layout:

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
  Forward slashes and backslashes both work.
- **Absolute includes** (`"\shared\lib\defs.hpp"`, which start with a backslash) name a path inside a PBO.
  dayzmake searches the folders that you give with `-i` for a file whose path ends in that path. It uses
  `$PBOPREFIX$` for this: the folder `shared` has the prefix `shared`, so `\shared\lib\defs.hpp` is
  `shared/lib/defs.hpp`. dayzmake does not search the current directory. It only checks the exact path there, so it
  is safe to run dayzmake in a big folder.

Without `-i`, dayzmake does not find the absolute include. The error names the file and the line:

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

The preprocessor replaced `ITEM_SIZE` and `BASE_WEIGHT`. `LABEL(Spare part)` became a quoted string through `#x`.
The two include lines left blank lines. The parser calculates `__EVAL(2*2)` later, when it reads the config.

If an include matches only when dayzmake ignores the case, dayzmake uses the file and gives a warning
(`include-case`). The game ignores the case of these names, but your Linux file system does not:

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

The output has the `__EVAL` result, no macros and no comments. Use `derapify` to read the `config.bin` of any PBO
(`dayzmake cat some.pbo config.bin | dayzmake derapify`). If you leave out the file arguments, both commands read
standard input and write standard output. You can use them in pipelines.

You seldom need `rapify` yourself. `build` rapifies each `.cpp` and `.rvmat` in the folder and gives the include
folders to the parser (`dayzmake build -i ../shared mymod mymod.pbo`).

## Mistakes the parser reports

dayzmake accepts an unquoted value and gives a warning. It supports `delete` and `+=`:

```text
$ dayzmake rapify warn.cpp w.bin        # class A { name = hello; z = ; };
In file warn.cpp:1: warning: String value "hello" is not quoted properly. [unquoted-string]
In file warn.cpp:1: warning: Entry "z" has no value; it is stored as an empty string. [empty-value]
$ dayzmake rapify -w unquoted-string -w empty-value warn.cpp -f w.bin
```

A syntax error stops the command:

```text
$ dayzmake rapify bad.cpp        # class A { x = 1   (no final newline)
error: In line bad.cpp:1:

  class A { x = 1
                 ^

Unexpected token "\n", expected: one of ".", ";", ['0'..='9'], ['e' | 'E'], [_]
```

A macro call with the wrong number of arguments gives a warning that names the line:

```text
$ dayzmake rapify mac.cpp -f /dev/null        # #define M(a,b) a+b / class A { x = M(1); };
In file mac.cpp:2: warning: Macro "M" expects 2 argument(s), got 1 [macro-arg-count]
In file mac.cpp:2: warning: Entry "x" has no value; it is stored as an empty string. [empty-value]
```

For every option, see the [reference](../reference/config.md).
