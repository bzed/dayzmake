# Keys and signatures: keygen, sign, verify

A DayZ server accepts a PBO from a client only if the PBO has a signature (`.bisign`). The signature must come from
a private key, and the public key (`.bikey`) must be in the `keys` folder of the server. dayzmake uses the file
formats of Bohemia: a 1024-bit RSA key pair and version 2 or 3 signatures. dayzmake does all of this in Rust and
uses no system libraries.

Key files and signatures are untrusted input, like every other file. dayzmake checks their lengths and the key name
before it makes a file name from the key name.

## dayzmake keygen

Generate a key pair.

<div class="synopsis">

```text
dayzmake keygen [-v] [-f] <keyname>
```

</div>

| Argument | Meaning |
|---|---|
| `<keyname>` | The path of the key without extension. The file name part is the name of the key. dayzmake stores it in both files, and it appears in signature names. Use a name such as `mymod_1` or your own handle. |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite existing key files. |

Behaviour:

- It writes `<keyname>.biprivatekey` and `<keyname>.bikey` next to each other. The folder of `<keyname>` must exist.
- Only the owner can read the private key (mode `0600`). Do not put it in version control. Do not publish it.
  Publish the `.bikey` with the mod.
- Without `-f`, an existing file of either name is an error, and dayzmake writes nothing.
- The key size is 1024 bits. DayZ expects this size.

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
| `<pbo>` | The PBO to sign. dayzmake only reads it. |
| `<signature>` | The signature file to write. Default: next to the PBO, `<pbo name>.<keyname>.bisign` (`mymod.pbo` signed with `mykey` gives `mymod.pbo.mykey.bisign`). |

| Option | Default | Meaning |
|---|---|---|
| `-v`, `--verbose` | off | Verbose output. |
| `-f`, `--force` | off | Overwrite an existing signature file. |
| `--v2` | off | Write an older version 2 signature instead of version 3. |

Behaviour:

- A version 3 signature covers the script and text-like entries (`.c`, `.sqf`, `.hpp`, `.cfg`, ...). Current DayZ
  versions check these entries. A version 2 signature covers everything except textures, models and sounds. You
  need it only for old servers or tools.
- dayzmake refuses a key name that you cannot use as a part of a file name (path separators, `:`, control
  characters). In that case, give `<signature>` in the command.
- dayzmake writes the signature atomically.
- `build -k` and `pack -k` do the same after they pack the PBO.

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

- If the signature is valid, dayzmake prints nothing and exits with status 0.
- If the signature is not valid, dayzmake exits with status 1 and prints an error. The signature stores the key
  that made it, and dayzmake compares this key first, so it reports a wrong key as a wrong key. If the PBO changed
  after signing, dayzmake prints the first hash that is different, with the signed value and the real value.
- dayzmake reads the version of the signature (2 or 3) from the signature file.

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
which have no relation to BI keys).
