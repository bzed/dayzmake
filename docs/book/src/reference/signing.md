# Keys and signatures: keygen, sign, verify

DayZ servers accept a PBO from a client only when it carries a signature (`.bisign`) made with a private key
whose public key (`.bikey`) is in the server's `keys` folder. dayzmake uses Bohemia's file formats: a 1024-bit RSA
key pair and version 2 or 3 signatures. Everything is implemented in Rust, with no system libraries.

Key files and signatures are untrusted input like every other file: dayzmake checks their lengths and the key
name before it writes a file name from it.

## dayzmake keygen

Generate a key pair.

<div class="synopsis">

```text
dayzmake keygen [-v] [-f] <keyname>
```

</div>

| Argument | Meaning |
|---|---|
| `<keyname>` | The path of the key without extension. The file name part is the name of the key (it is stored in both files and appears in signature names), so use a name such as `mymod_1` or your own handle. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite existing key files. |

Behaviour:

- Writes `<keyname>.biprivatekey` and `<keyname>.bikey` next to each other; the folder of `<keyname>` must exist.
- The private key is created readable by its owner only (mode `0600`). Keep it out of version control and never
  publish it; publish the `.bikey` with the mod.
- Without `-f`, an existing file of either name is an error and nothing is written.
- The key size is 1024 bits, which is what DayZ expects.

Examples:

```text
$ dayzmake keygen release
$ ls -l release.*
-rw-r--r-- 1 me me 160 Nov 16 15:39 release.bikey
-rw------- 1 me me 608 Nov 16 15:39 release.biprivatekey
$ dayzmake keygen release
error: Target file "release.biprivatekey" already exists. Use --force to overwrite.
```

See also: [sign](#dayzmake-sign), [A first mod](../guides/first-mod.md).

## dayzmake sign

Sign a PBO with a private key.

<div class="synopsis">

```text
dayzmake sign [-v] [-f] [--v2] <privatekey> <pbo> [<signature>]
```

</div>

| Argument | Meaning |
|---|---|
| `<privatekey>` | The `.biprivatekey`. |
| `<pbo>` | The PBO to sign; it is only read. |
| `<signature>` | The signature file to write. Default: next to the PBO, `<pbo name>.<keyname>.bisign` (`mymod.pbo` signed with `mykey` gives `mymod.pbo.mykey.bisign`). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing signature file. |
| `--v2` | off | Write an older version 2 signature instead of version 3. |

Behaviour:

- A version 3 signature covers the script and text-like entries (`.c`, `.sqf`, `.hpp`, `.cfg`, ...), which is
  what current DayZ versions check. A version 2 signature covers everything except textures, models and sounds,
  and is only needed for old servers or tools.
- A key name that is not usable as a file name part (path separators, `:`, control characters) is refused;
  pass `<signature>` explicitly then.
- The signature is written atomically.
- `build -k` and `pack -k` do the same after packing.

Examples:

```text
$ dayzmake sign mykey.biprivatekey mymod.pbo
$ ls mymod*
mymod.pbo  mymod.pbo.mykey.bisign
$ dayzmake sign mykey.biprivatekey mymod.pbo
error: Target file "mymod.pbo.mykey.bisign" already exists. Use --force to overwrite.
$ dayzmake sign -f --v2 mykey.biprivatekey mymod.pbo
```

```text
$ dayzmake sign nokey.biprivatekey sg.pbo
error: Failed to open private key "nokey.biprivatekey":
No such file or directory (os error 2)
```

See also: [verify](#dayzmake-verify), [keygen](#dayzmake-keygen).

## dayzmake verify

Check the signature of a PBO.

<div class="synopsis">

```text
dayzmake verify [-v] <publickey> <pbo> [<signature>]
```

</div>

| Argument | Meaning |
|---|---|
| `<publickey>` | The `.bikey`. |
| `<pbo>` | The PBO to check. |
| `<signature>` | The signature file. Default: `<pbo name>.<keyname>.bisign`, where the key name is stored in the public key. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |

Behaviour:

- Prints nothing and exits with status 0 when the signature is valid.
- Exits with status 1 and an error when it is not. The key a signature was made with is stored in the signature
  and compared first, so a wrong key is reported as such. If the PBO changed after signing, the first hash that
  differs is printed with the signed and the real value.
- The version of the signature (2 or 3) is read from the signature file.

Examples:

```text
$ dayzmake verify release.bikey sg.pbo
$ echo $?
0
```

A key that did not make the signature:

```text
$ dayzmake verify other.bikey mymod.pbo mymod.pbo.mykey.bisign
error: The signature was made with a different key (signature: mykey, 1024 bits; this key: other, 1024 bits).
```

A PBO whose `a.c` was changed after it was signed:

```text
$ dayzmake verify release.bikey sg2.pbo
error: Hash 1 doesn't match
Signed hash: 025f08824316bcd409d45bcef929192c2b16375b
Real hash:   e2ac5c2b8005409dbca48bc11e9424f2d574f62a
```

A signature file that does not exist:

```text
$ dayzmake verify release.bikey sg.pbo missing.bisign
error: Failed to open signature "missing.bisign":
No such file or directory (os error 2)
```

See also: [sign](#dayzmake-sign), [Verifying downloads](../verifying.md) (for the GPG signatures of dayzmake itself,
which are unrelated to BI keys).
