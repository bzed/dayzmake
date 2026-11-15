# Pure-Rust RSA for BI Keys and Signatures: Investigation and Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace OpenSSL in `dayzmake-core::sign` (and the last OpenSSL use in the CLI crate) with reviewed pure-Rust
crypto, with signatures **byte-identical** to the OpenSSL implementation, so dayzmake builds without a system
OpenSSL on every platform.

**Architecture:** BI signing is raw ("textbook") RSA over BI's own padded SHA-1 hashes: `sig = pad(hash)^d mod n`,
verification `sig^e mod n == pad(hash)`. The RustCrypto `rsa` 0.10 crate's `hazmat` module provides exactly these raw
operations (`rsa_decrypt` with blinding, `rsa_encrypt`) on constant-time `crypto-bigint` integers, and key generation
through `crypto-primes`. OpenSSL stays as a **dev-dependency only**, for differential tests that prove identical
results; production builds have no OpenSSL.

**Tech Stack:** `rsa` 0.10 (`hazmat`, no default features), `crypto-bigint` ≥ 0.7.11 (via `rsa`), `sha1` (added by the
workspace-split plan, Task 11), `rand_chacha` (dev, seeded RNG for deterministic key-generation tests), `openssl` (dev).

**Spec:** this document's *Investigation* section and the user's request (2026-11-15): "investigate what needs to be
done to handle [RSA signing] in pure rust, if the results match the openssl variant … ensure that the crypto code is
trustworthy".

## Investigation (2026-11-15)

### What dayzmake uses OpenSSL for

| Use | Where | OpenSSL API |
|---|---|---|
| SHA-1 of the PBO trailer | `pbo.rs` | `Hasher` — **being replaced by `sha1`** in the workspace-split plan, Task 11 |
| SHA-1 of names/files for signatures | `sign.rs` `namehash`, `filehash`, `generate_hashes` | `Hasher`, `DigestBytes` |
| Key generation (1024-bit, e = 65537) | `BIPrivateKey::generate` | `Rsa::generate`, then n, p, q, dmp1, dmq1, iqmp, d |
| Signing | `BIPrivateKey::sign` | `BigNum::mod_exp(hash, d, n)` × 3 (no CRT, no blinding) |
| Verification | `BIPublicKey::verify` | `BigNum::mod_exp(sig, e, n)` × 3 |
| Key/signature I/O | `read`/`write`, `bignum`, `write_bignum` | `BigNum::from_slice`, `to_vec` (big-endian; the files store little-endian) |
| API index cache key | `src/api/cache.rs:165` | `openssl::hash::hash` |

### Do pure-Rust results match?

- **Signatures: yes, by construction.** RSA signing here is deterministic: `m^d mod n` has exactly one value. The
  `rsa` crate computes it with CRT and random blinding, which changes the computation, never the result
  (`rsa_decrypt` multiplies by `r^e`, exponentiates, and divides `r` out; CRT recombination gives the same residue).
  Given the same key and the same padded hashes, the `.bisign` bytes are identical. This must still be proven by tests
  (below), not assumed.
- **Verification: yes**, same argument for `sig^e mod n`.
- **Key generation: not byte-comparable.** Keys are random. OpenSSL computes `d` modulo λ(n) or φ(n) depending on
  version; `rsa` 0.10 uses φ(n) (`compute_private_exponent_euler_totient`). Both produce valid keys that sign
  identically to themselves. What must match is the **file format** and **interoperability with BI's tools**
  (`DSSignFile.exe`, `DSCheckSignatures.exe`).
- **SHA-1:** identical (proven for `sha1` vs OpenSSL in Task 11 of the split plan).

### Candidates

| Option | Verdict |
|---|---|
| **`rsa` 0.10 (RustCrypto)** — MIT/Apache-2.0, `#![forbid(unsafe_code)]`-style (0 `unsafe` in `src/`), MSRV 1.87, constant-time `crypto-bigint` backend, blinding, zeroizes private keys on drop, `hazmat` raw ops, `PrivateKey::from_components` validates imported keys. RUSTSEC-2023-0071 (Marvin timing attack) is fixed in ≥ 0.10.0. | **Recommended.** It is the only maintained pure-Rust crate with raw RSA ops and key generation. |
| `crypto-bigint` + `crypto-primes` directly | Rejected: we would write our own modexp/CRT/blinding/key generation — more self-written crypto, less trustworthy. |
| `num-bigint` / `num-bigint-dig` (`rsa` 0.9) | Rejected: not constant-time; `rsa` 0.9 is affected by RUSTSEC-2023-0071. |
| `ring`, `aws-lc-rs` | Rejected: no raw RSA private op (ring) or not pure Rust (aws-lc-rs is C). |

Advisories to watch (local RustSec DB, 2026-11-15): RUSTSEC-2023-0071 (`rsa` < 0.10), RUSTSEC-2026-0387
(`crypto-bigint` < 0.7.6, timing variability; `rsa` 0.10 requires ≥ 0.7.11), RUSTSEC-2026-0097 (`rand` unsoundness
with a custom logger; patched ≥ 0.10.1 / 0.9.3 / 0.8.6 — avoid `rand`, use `rand_core` + `getrandom`).

### Does it make sense?

Yes, with conditions:

- **Gains:** no system OpenSSL anywhere (drops `libssl-dev` from CI/Docker, the Windows vcpkg static-OpenSSL step and
  the README's OpenSSL setup section); Náves and other consumers build `dayzmake-core` with `cargo build` alone;
  constant-time arithmetic and blinding (today: OpenSSL `mod_exp` on `d` without the constant-time flag, i.e. *less*
  side-channel protection than the replacement); private keys zeroized on drop.
- **Costs/risks:** about ten new crates (`rsa`, `crypto-bigint`, `crypto-primes`, `rand_core`, `getrandom`,
  `zeroize`, `const-oid`, `digest`, `signature`, `subtle`/`ctutils`); `rsa` 0.10.0 is a young release line; raw RSA
  is a "hazmat" API (but BI's format *is* raw RSA — there is no padding scheme to choose); `PublicKey::MIN_SIZE` is
  1024 bits while dayzmake accepts keys from 288 bits (`check_key_length`) — smaller keys need
  `PublicKey::new_with_size_limits` for verification, and generation stays at 1024.
- **Condition:** byte-identical signatures for every fixture and a differential suite against OpenSSL, plus one manual
  interoperability check with BI's tools, before OpenSSL is removed.

## Global Constraints

- Runs **after** the workspace-split plan (needs `dayzmake-core`, the `sign` feature and `sha1` from its Task 11).
  Branch `feat/pure-rust-rsa` from the split branch head (or `main` once merged).
- All `AGENTS.md` rules, especially *Dependencies* (strict review, written into the commit), *Untrusted input never
  panics* (key and signature files are hostile), and *Secrets* (no key material in errors or logs).
- `rsa` with `default-features = false, features = ["std", "getrandom", "hazmat"]` — no `encoding`/PKCS features.
  No `rand` crate in the dependency tree (RUSTSEC-2026-0097); seeded RNG in tests via `rand_chacha` + `rand_core`.
- Our code contains no modular arithmetic of its own: every operation on secret numbers goes through `rsa`
  (`rsa_decrypt` with an RNG for blinding) or `crypto-bigint` constant-time APIs.
- Test keys are generated at test time from fixed seeds (`rand_chacha`); no new key files in the repo.
- `.bisign`, `.bikey`, `.biprivatekey` formats are byte-for-byte unchanged; every BI fixture under
  `tests/fixtures/bi/sign/` stays byte-identical when re-signed.
- `openssl` becomes a dev-dependency of `dayzmake-core` only (differential tests); `cargo tree -e normal` of both crates
  shows no `openssl`/`openssl-sys`.
- `make check` passes after every task; Windows CI (tests + clippy) is green before the PR is marked ready.

## Review Focus

1. **Hostile `.biprivatekey`:** inconsistent components (p·q ≠ n, d not matching e, wrong dmp1/dmq1/iqmp), zero or
   one-valued numbers, or lengths not matching the header must be errors, never panics, and never sign with garbage.
   `PrivateKey::from_components` validates; we additionally compare the stored dmp1/dmq1/iqmp with the recomputed
   ones (Task 2).
2. **Keys smaller or larger than `rsa`'s defaults** (288..1024 and > 8192 bits from foreign files): verification of
   such signatures must keep working where it works today, or fail with a clear error (Task 3).
3. **Signatures whose value is ≥ n** (hostile `.bisign`): verify must return an error, not panic (Task 3).
4. **Error messages must not contain key material** (no `d`, `p`, `q` in `Display`/`Debug` output) (Task 2).
5. **Generated keys must be accepted by BI's tools** — manual check, recorded in the PR (Task 5).

---

### Task 1: Differential safety net (before any switch)

**Files:** Create `crates/dayzmake-core/tests/sign_differential.rs`; modify `crates/dayzmake-core/Cargo.toml`
(dev-deps `rand_chacha`, `rand_core`; `openssl` is still a normal dependency at this point).

- [ ] **Step 1:** Tests that pin today's results with the current OpenSSL code, so the switch can be measured:
  - `fixture_signatures_are_reproduced`: for every PBO in `tests/fixtures/bi/sign/` with a BI-made `.bisign`, signing
    with `armk.biprivatekey` (V2 and V3 as the fixture says) gives the fixture bytes.
  - `seeded_keys_sign_like_an_independent_implementation`: **keys are generated at test time from fixed seeds, never
    committed** (maintainer decision 2026-11-15). A test helper generates 8 keys with
    `rsa::RsaPrivateKey::new_with_exp(&mut ChaCha20Rng::seed_from_u64(seed), bits, 65537)` (seeds 1–8; six 1024-bit,
    one 2048, one 4096) and serialises them into `.biprivatekey` bytes (BI layout: name, header, n, p, q, dmp1, dmq1,
    iqmp, d little-endian — the same layout `BIPrivateKey::write` produces; the helper is test code, checked by
    reading the bytes back with `BIPrivateKey::read` and writing them again byte-identically). For each key, sign
    the golden PBO (V2 and V3) with dayzmake and compare with the signature computed **independently** in the test
    from the same padded hashes by the *other* library: before the switch dayzmake uses OpenSSL and the test computes
    `rsa::hazmat::rsa_decrypt`; after the switch dayzmake uses `rsa` and the test computes OpenSSL
    `BigNum::mod_exp(hash, d, n)`. `rsa`, `rand_chacha` and `rand_core` are dev-dependencies in this task; `rsa`
    becomes a normal dependency in Task 2. The padded hashes come from a `pub(crate)`/`#[doc(hidden)]`-free path:
    expose nothing new; recompute them in the test from the PBO with `sha1` and BI's padding (a short, documented
    helper), and assert that verifying dayzmake's signature with the public key yields exactly those padded hashes.
  - `verify_accepts_and_rejects`: each fixture signature verifies; one flipped bit in the signature, in the PBO data,
    and in the key's `n` each fails verification with an error.
- [ ] **Step 2:** Run → PASS on the OpenSSL code. Commit `test: pin BI signature results before the RSA switch`.
  No key material is committed; only the existing throwaway `armk` key stays in the repo.

### Task 1b: Verify recomputes the PBO's SHA-1 (#46)

**Files:** `crates/dayzmake-core/src/pbo.rs` (`PBO::read`, `trailer_checksum`), `crates/dayzmake-core/src/sign.rs`
(`generate_hashes`, `sign`, `verify`), tests, fuzz seed, README `## Changes`.

Hash 1 of a BI signature is the SHA-1 of the whole PBO, but `PBO::read` keeps the stored trailer and never checks it, so
`verify` accepts PBOs whose config.bin, V2-excluded files or header were changed (issue #46 has the repro). Fix:
`PBO::read` hashes the bytes as it reads them (header and data, everything before the trailer) and keeps the computed
digest next to the stored one; `generate_hashes` uses the computed digest; `sign` and `verify` fail with a clear error
(naming the checksum mismatch, no key material) when the stored trailer differs from the computed one. Other readers
(`inspect`, `unpack`, the API index) keep working on such files; `inspect` may show the mismatch. Public API additive.
Tests first: each tampering case of #46 (config.bin byte, `.txt`, `.rvmat`, a header extension; V3 fixture signature,
and a V2 signature for the `.rvmat`) → verify fails; unmodified fixtures still verify; sign of a PBO with a wrong
trailer fails. Update `verify_accepts_and_rejects` (Task 1) to flip bits in config.bin and the header too, and drop the
comment that calls the stored-trailer behaviour intended. Fuzz seed: a PBO with a wrong trailer. Commit
`fix: verify recomputes the PBO checksum instead of trusting the trailer (#46)`.

### Task 2: Private and public keys on `rsa` types

**Files:** `crates/dayzmake-core/src/sign.rs`, `Cargo.toml`.

**Interfaces:** `BIPrivateKey` keeps its public API (`read`, `write`, `generate`, `to_public_key`, `sign`, `name`);
internally it holds `rsa::RsaPrivateKey` (0.10 name: check `rsa::PrivateKey`/`RsaPrivateKey`) plus the name and
length. `BIPublicKey` holds `rsa::RsaPublicKey` (or `n`/`e` as `BoxedUint` where the size limits require it).

- [ ] **Step 1:** Tests (hostile keys, Review Focus 1 and 4): `private_key_with_wrong_n_is_rejected`,
  `private_key_with_wrong_crt_values_is_rejected`, `zero_and_one_components_are_rejected`,
  `key_errors_contain_no_key_material` (format the error of a broken key file; assert none of the hex/decimal forms of
  its `d`, `p`, `q` appear).
- [ ] **Step 2:** Implement reading: little-endian file bytes → `BoxedUint` with the key's bit precision; build the key
  with `PrivateKey::from_components(n, e, d, vec![p, q])`; compare its precomputed dp/dq/qinv with the file's
  dmp1/dmq1/iqmp (error on mismatch). Writing: the stored components, little-endian, padded to the same sizes as
  today (`write_bignum`).
- [ ] **Step 3:** Run Task 1 + new tests → PASS. Commit `refactor: BI keys on pure-Rust RSA types` (body: crate review
  of `rsa`, `crypto-bigint`, `crypto-primes`, `rand_core`, `getrandom`, `zeroize` and the rest of the new tree —
  source read, `unsafe` count, advisories, maintenance, licences, MSRV). Mention that `Cargo.lock` lists `rsa`'s optional
  `pkcs1`/`pkcs8`/`der`/`spki`/`pem-rfc7468`/`serdect`, which are not built (`cargo tree -i pkcs1` is empty).

### Task 2b: Preprocessor stack overflow found by fuzzing (#51)

Not RSA work, but `make fuzz` (part of `make check`) hits it on every branch. Find the recursive path of the #51 input
(macro expansion with token pasting / stringising / argument pre-expansion in `crates/dayzmake-core/src/preprocess.rs`)
and apply a depth limit there: a normal error naming the macro (through `printable`), never a stack overflow. Minimise
the input; regression test in `crates/dayzmake-core/tests/` and a `config_parse` fuzz seed in the same commit. Commit
`fix: macro expansion has a depth limit on every path (#51)`.

### Task 3: Signing and verification

**Files:** `crates/dayzmake-core/src/sign.rs`.

- [ ] **Step 1:** Tests: `signature_value_not_below_n_is_an_error` (Review Focus 3), `small_key_signatures_verify`
  (a 512-bit throwaway key fixture signed with today's code, Review Focus 2) or a documented error if `rsa` cannot
  represent it. In `verify_accepts_and_rejects`, the flipped-`n` case must treat a key that `BIPublicKey::read`
  rejects as a pass (an even `n` may be refused at read time by `rsa`), not `unwrap` the read.
- Also (Task 2 review M3): every buffer holding secret key bytes (the `read_uint` buffer, the outputs of
  `to_be_bytes_trimmed_vartime` in `write_uint`/`same_value`, the d copies) is `zeroize::Zeroizing`; the OpenSSL
  copies of d disappear with this task.
- [ ] **Step 2:** `sign`: `rsa::hazmat::rsa_decrypt(Some(&mut OsRng), &key, &padded_hash)` for each of the three
  hashes (blinding on). `verify`: `rsa::hazmat::rsa_encrypt(&public_key, &sig)` and constant-time comparison with the
  padded hash (`subtle::ConstantTimeEq` or `BoxedUint` equality — public values, plain `==` is acceptable; say which).
  SHA-1 through `sha1` (identical, proven in the split plan).
- [ ] **Step 3:** Run Task 1 + new tests → PASS, all fixtures byte-identical. Commit `feat: sign and verify BI
  signatures without OpenSSL`.

### Task 4: Key generation

**Files:** `crates/dayzmake-core/src/sign.rs`.

- [ ] **Step 1:** Tests with a seeded `ChaCha20Rng` through an internal `generate_with_rng(length, name, rng)`:
  `generated_key_is_valid` (n has `length` bits, e = 65537, p·q = n, e·d ≡ 1 mod φ(n), CRT values consistent, the file
  round-trips byte-identically), `generated_key_signs_and_verifies` (sign the golden PBO, verify with the derived
  public key; and verify the same signature with the **OpenSSL** implementation kept in the differential test helper),
  `seeded_generation_is_deterministic` (same seed → same key file bytes).
- [ ] **Step 2:** `generate` uses the OS RNG (`getrandom`); `generate_with_rng` is `pub(crate)` for the tests.
- [ ] **Step 3:** Run → PASS. Commit `feat: generate BI keys without OpenSSL`.

### Task 5: Remove OpenSSL, interop check, docs

**Files:** both `Cargo.toml`s (openssl → dev-dependency of core only; remove from the root crate after switching
`src/api/cache.rs` to `sha1`/`sha2` — check which digest the cache key uses and keep its output identical, or bump the
cache format version if it must change), `.github/workflows/ci.yml` (drop `libssl-dev`, `pkg-config` where only
OpenSSL needed them, the Windows vcpkg/`OPENSSL_*` steps), `Dockerfile`, `README.md` (build requirements),
`deny.toml` if needed.

- [ ] **Step 1:** `cargo tree -p dayzmake -e normal | grep -i openssl` → nothing;
  `cargo tree -p dayzmake-core -e normal | grep -i openssl` → nothing. `make check`, Windows CI green.
- [ ] **Step 2 (manual, maintainer):** with BI's DayZ Tools under Proton: a key from the new `dayzmake keygen` signs a
  PBO with `DSSignFile.exe`; `DSCheckSignatures.exe -deep` accepts a PBO signed by the new `dayzmake sign`; record
  the result in the PR. Add a `#[ignore = "needs DayZ Tools"]` test that runs both if the tools are configured (like
  `tests/binarize_proton.rs`).
- [ ] **Step 3:** Commit `build: drop OpenSSL` (body: interop result, dependency tree diff).
