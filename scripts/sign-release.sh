#!/usr/bin/env bash
# Writes SHA256SUMS and SHA512SUMS for the files of a release folder and signs both with the release key.
#
#   scripts/sign-release.sh <dist-dir>
#
# Environment:
#   RELEASE_GPG_PRIVATE_KEY  the ASCII-armoured secret key (gpg --armor --export-secret-keys)
#   RELEASE_GPG_PASSPHRASE   its passphrase
#   RELEASE_GPG_PUBLIC_KEY   optional: the published public key to check the signatures against
#                            (default: docs/book/src/release-key.asc next to this script's folder)
#
# In <dist-dir> it creates SHA256SUMS, SHA512SUMS (bare file names, sorted, one line for every regular file that was
# there before the script ran) and the detached ASCII signatures SHA256SUMS.asc and SHA512SUMS.asc. It fails, and
# leaves nothing behind in <dist-dir>, when a secret is missing, the key is expired or the passphrase is wrong. It uses a
# private GNUPGHOME that is removed on exit, and passes the passphrase on a file descriptor, never on a command line.
# At the end both signatures are verified in a second, fresh GNUPGHOME that holds only the published public key,
# and the script fails when the signing key is not that key.
set -euo pipefail

die() {
    echo "sign-release: $*" >&2
    exit 1
}

[ "$#" -eq 1 ] || die "usage: $0 <dist-dir>"
dist=$1
[ -d "$dist" ] || die "$dist is not a directory"
[ -n "${RELEASE_GPG_PRIVATE_KEY:-}" ] || die "RELEASE_GPG_PRIVATE_KEY is empty or not set"
[ -n "${RELEASE_GPG_PASSPHRASE:-}" ] || die "RELEASE_GPG_PASSPHRASE is empty or not set"

dist=$(cd "$dist" && pwd)

# the files to cover: every regular file now, in byte order, without the outputs of an earlier run
files=()
while IFS= read -r -d '' name; do
    case $name in
        SHA256SUMS | SHA256SUMS.asc | SHA512SUMS | SHA512SUMS.asc) ;;
        *) files+=("$name") ;;
    esac
done < <(cd "$dist" && find . -maxdepth 1 -type f -printf '%P\0' | LC_ALL=C sort -z)
[ "${#files[@]}" -gt 0 ] || die "$dist has no files to checksum"

work=$(mktemp -d "${TMPDIR:-/tmp}/sign-release.XXXXXX")
export GNUPGHOME="$work/gnupg"
mkdir -m 700 "$GNUPGHOME"
stage="$work/stage"
mkdir "$stage"
# stop the agent that gpg started for the private home, then remove everything
trap 'gpgconf --homedir "$GNUPGHOME" --kill gpg-agent >/dev/null 2>&1 || true; rm -rf "$work"' EXIT

# run a gpg command; on failure print what gpg said (its messages carry no secrets) and stop
gpg_err="$work/gpg.err"
gpg_step() {
    local what=$1
    shift
    if ! "$@" 2>"$gpg_err"; then
        echo "sign-release: $what failed; gpg said:" >&2
        sed 's/^/    /' "$gpg_err" >&2
        exit 1
    fi
}

public_key=${RELEASE_GPG_PUBLIC_KEY:-$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)/docs/book/src/release-key.asc}
[ -f "$public_key" ] || die "the published public key $public_key does not exist (set RELEASE_GPG_PUBLIC_KEY)"

import_secret() { printf '%s\n' "$RELEASE_GPG_PRIVATE_KEY" | gpg --batch --quiet --import; }
gpg_step "importing RELEASE_GPG_PRIVATE_KEY" import_secret
fingerprint=$(gpg --batch --list-secret-keys --with-colons 2>/dev/null | awk -F: '$1 == "fpr" {print $10; exit}')
[ -n "$fingerprint" ] || die "RELEASE_GPG_PRIVATE_KEY holds no secret key"

# everything is made in a staging folder and moved into <dist-dir> only when all checks passed
for algo in 256 512; do
    sums="SHA${algo}SUMS"
    (cd "$dist" && "sha${algo}sum" -- "${files[@]}" >"$stage/$sums")
    # the passphrase comes in on file descriptor 3 (a here-string), not as an argument
    sign() {
        gpg --batch --quiet --pinentry-mode loopback --passphrase-fd 3 --local-user "$fingerprint" \
            --armor --detach-sign --output "$stage/$sums.asc" "$stage/$sums" 3<<<"$RELEASE_GPG_PASSPHRASE"
    }
    gpg_step "signing $sums" sign
done

# self-check: the sums match the files, as a user would check them
for algo in 256 512; do
    sums="SHA${algo}SUMS"
    (cd "$dist" && "sha${algo}sum" -c --quiet "$stage/$sums") || die "$sums does not match the files"
done

# the signatures must verify with the published public key alone, and that key must be the signing key
verify_home="$work/verify-gnupg"
mkdir -m 700 "$verify_home"
trap 'gpgconf --homedir "$GNUPGHOME" --kill gpg-agent >/dev/null 2>&1 || true; gpgconf --homedir "$verify_home" --kill gpg-agent >/dev/null 2>&1 || true; rm -rf "$work"' EXIT
published=$(gpg --batch --with-colons --show-keys "$public_key" 2>"$gpg_err" | awk -F: '$1 == "fpr" {print $10; exit}') ||
    true
[ -n "$published" ] || die "$public_key holds no public key"
[ "$published" = "$fingerprint" ] ||
    die "the signing key ($fingerprint) is not the published key ($published in $public_key)"
verify_all() {
    gpg --homedir "$verify_home" --batch --quiet --import "$public_key" &&
        gpg --homedir "$verify_home" --batch --quiet --verify "$stage/SHA256SUMS.asc" "$stage/SHA256SUMS" &&
        gpg --homedir "$verify_home" --batch --quiet --verify "$stage/SHA512SUMS.asc" "$stage/SHA512SUMS"
}
gpg_step "verifying the signatures with the published public key" verify_all

mv -f "$stage"/SHA256SUMS "$stage"/SHA256SUMS.asc "$stage"/SHA512SUMS "$stage"/SHA512SUMS.asc "$dist"/
echo "signed with key ${fingerprint}: SHA256SUMS and SHA512SUMS for ${#files[@]} file(s)"
