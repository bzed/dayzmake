#!/usr/bin/env bash
# Writes SHA256SUMS and SHA512SUMS for the files of a release folder and signs both with the release key.
#
#   scripts/sign-release.sh <dist-dir>
#
# Environment:
#   RELEASE_GPG_PRIVATE_KEY  the ASCII-armoured secret key (gpg --armor --export-secret-keys)
#   RELEASE_GPG_PASSPHRASE   its passphrase
#
# In <dist-dir> it creates SHA256SUMS, SHA512SUMS (bare file names, sorted, one line for every regular file that was
# there before the script ran) and the detached ASCII signatures SHA256SUMS.asc and SHA512SUMS.asc. It fails, and
# leaves nothing behind in <dist-dir>, when a secret is missing, the key is expired or the passphrase is wrong. It uses a
# private GNUPGHOME that is removed on exit, and passes the passphrase on a file descriptor, never on a command line.
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

printf '%s\n' "$RELEASE_GPG_PRIVATE_KEY" | gpg --batch --quiet --import 2>/dev/null ||
    die "RELEASE_GPG_PRIVATE_KEY is not a usable secret key"
fingerprint=$(gpg --batch --list-secret-keys --with-colons 2>/dev/null | awk -F: '$1 == "fpr" {print $10; exit}')
[ -n "$fingerprint" ] || die "RELEASE_GPG_PRIVATE_KEY holds no secret key"

# everything is made in a staging folder and moved into <dist-dir> only when all checks passed
for algo in 256 512; do
    sums="SHA${algo}SUMS"
    (cd "$dist" && "sha${algo}sum" -- "${files[@]}" >"$stage/$sums")
    # the passphrase comes in on file descriptor 3 (a here-string), not as an argument
    gpg --batch --quiet --pinentry-mode loopback --passphrase-fd 3 --local-user "$fingerprint" \
        --armor --detach-sign --output "$stage/$sums.asc" "$stage/$sums" 3<<<"$RELEASE_GPG_PASSPHRASE" 2>/dev/null ||
        die "gpg could not sign $sums: the key is expired or RELEASE_GPG_PASSPHRASE is wrong"
done

# self-check: the signatures verify and the sums match the files, as a user would check them
for algo in 256 512; do
    sums="SHA${algo}SUMS"
    gpg --batch --quiet --verify "$stage/$sums.asc" "$stage/$sums" 2>/dev/null || die "the signature of $sums does not verify"
    (cd "$dist" && "sha${algo}sum" -c --quiet "$stage/$sums") || die "$sums does not match the files"
done

mv -f "$stage"/SHA256SUMS "$stage"/SHA256SUMS.asc "$stage"/SHA512SUMS "$stage"/SHA512SUMS.asc "$dist"/
echo "signed with key ${fingerprint}: SHA256SUMS and SHA512SUMS for ${#files[@]} file(s)"
