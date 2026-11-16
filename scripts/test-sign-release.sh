#!/usr/bin/env bash
# Test of scripts/sign-release.sh with a throwaway key. Not part of CI; run it by hand after changing the script:
#   scripts/test-sign-release.sh
# Everything happens in a temporary folder below /tmp.
set -euo pipefail

here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
script="$here/sign-release.sh"
work=$(mktemp -d /tmp/sign-release-test.XXXXXX)
cleanup() {
    for home in "$work"/*-home*; do
        [ -d "$home" ] && gpgconf --homedir "$home" --kill gpg-agent >/dev/null 2>&1 || true
    done
    rm -rf "$work"
}
trap cleanup EXIT
failures=0

fail() {
    echo "FAIL: $*" >&2
    failures=$((failures + 1))
}

# make_key <home> <passphrase> [extra gpg options]: a throwaway ed25519 signing key; prints its armoured secret key.
make_key() {
    local home=$1 pass=$2
    shift 2
    mkdir -m 700 "$home"
    gpg --homedir "$home" --batch --quiet --pinentry-mode loopback --passphrase "$pass" "$@" \
        --quick-generate-key "dayzmake test key" ed25519 sign 1y 2>/dev/null
    gpg --homedir "$home" --batch --quiet --pinentry-mode loopback --passphrase "$pass" \
        --armor --export-secret-keys 2>/dev/null
}

make_dist() {
    mkdir -p "$1"
    printf 'first\n' >"$1/dayzmake-linux-x86_64.tar.gz"
    printf 'second\n' >"$1/dayzmake-windows-x86_64.zip"
}

secret=$(make_key "$work/key-home" "correct horse")
gpg --homedir "$work/key-home" --armor --export >"$work/public.asc" 2>/dev/null

# 1. success: two files in, sorted bare names in both sums files, valid signatures, sums check out
make_dist "$work/dist"
if ! RELEASE_GPG_PRIVATE_KEY=$secret RELEASE_GPG_PASSPHRASE="correct horse" "$script" "$work/dist" >"$work/ok.log" 2>&1; then
    fail "the script failed on a good key: $(cat "$work/ok.log")"
else
    for algo in 256 512; do
        sums="$work/dist/SHA${algo}SUMS"
        names=$(awk '{print $2}' "$sums" | tr '\n' ' ')
        [ "$names" = "dayzmake-linux-x86_64.tar.gz dayzmake-windows-x86_64.zip " ] ||
            fail "SHA${algo}SUMS lists '$names'"
        [ -f "$sums.asc" ] || fail "SHA${algo}SUMS.asc is missing"
        mkdir -m 700 "$work/verify-home-$algo"
        gpg --homedir "$work/verify-home-$algo" --batch --quiet --import "$work/public.asc" 2>/dev/null
        gpg --homedir "$work/verify-home-$algo" --batch --verify "$sums.asc" "$sums" 2>/dev/null ||
            fail "gpg --verify of SHA${algo}SUMS.asc failed"
        (cd "$work/dist" && "sha${algo}sum" -c "SHA${algo}SUMS" >/dev/null) ||
            fail "sha${algo}sum -c failed inside the download folder"
    done
    [ "$(find "$work/dist" -maxdepth 1 -type f | wc -l)" -eq 6 ] || fail "dist holds unexpected files"
fi

# 2. a second run does not list the sums or signatures of the first
if RELEASE_GPG_PRIVATE_KEY=$secret RELEASE_GPG_PASSPHRASE="correct horse" "$script" "$work/dist" >/dev/null 2>&1; then
    grep -q 'SHA' "$work/dist/SHA256SUMS" && fail "a second run lists its own outputs"
else
    fail "the second run failed"
fi

# 3. an empty passphrase fails and names the variable
make_dist "$work/dist2"
if RELEASE_GPG_PRIVATE_KEY=$secret RELEASE_GPG_PASSPHRASE='' "$script" "$work/dist2" >"$work/e1.log" 2>&1; then
    fail "an empty passphrase was accepted"
elif ! grep -q 'RELEASE_GPG_PASSPHRASE' "$work/e1.log"; then
    fail "the error for an empty passphrase does not name RELEASE_GPG_PASSPHRASE: $(cat "$work/e1.log")"
fi
[ ! -e "$work/dist2/SHA256SUMS.asc" ] || fail "signatures were left behind after a failure"

# 4. an empty key fails and names the variable
if RELEASE_GPG_PRIVATE_KEY='' RELEASE_GPG_PASSPHRASE=x "$script" "$work/dist2" >"$work/e2.log" 2>&1; then
    fail "an empty key was accepted"
elif ! grep -q 'RELEASE_GPG_PRIVATE_KEY' "$work/e2.log"; then
    fail "the error for an empty key does not name RELEASE_GPG_PRIVATE_KEY: $(cat "$work/e2.log")"
fi

# 5. a wrong passphrase fails
if RELEASE_GPG_PRIVATE_KEY=$secret RELEASE_GPG_PASSPHRASE="wrong" "$script" "$work/dist2" >"$work/e3.log" 2>&1; then
    fail "a wrong passphrase was accepted"
fi
[ ! -e "$work/dist2/SHA256SUMS.asc" ] || fail "signatures were left behind after a wrong passphrase"

# 6. an expired key fails (created in 2020 with a one day lifetime)
mkdir -m 700 "$work/expired-home"
gpg --homedir "$work/expired-home" --batch --quiet --pinentry-mode loopback --passphrase pw \
    --faked-system-time 20200101T000000 --quick-generate-key "expired test key" ed25519 sign 1d 2>/dev/null
expired=$(gpg --homedir "$work/expired-home" --batch --quiet --pinentry-mode loopback --passphrase pw \
    --armor --export-secret-keys 2>/dev/null)
if RELEASE_GPG_PRIVATE_KEY=$expired RELEASE_GPG_PASSPHRASE=pw "$script" "$work/dist2" >"$work/e4.log" 2>&1; then
    fail "an expired key was accepted"
fi
[ ! -e "$work/dist2/SHA256SUMS.asc" ] || fail "signatures were left behind after an expired key"

# 7. the secrets never show up in the output
if grep -q -e 'correct horse' -e 'BEGIN PGP PRIVATE' "$work/ok.log" "$work"/e*.log; then
    fail "a secret appeared in the output"
fi

# 8. an empty or missing dist folder fails
mkdir "$work/empty"
if RELEASE_GPG_PRIVATE_KEY=$secret RELEASE_GPG_PASSPHRASE="correct horse" "$script" "$work/empty" >/dev/null 2>&1; then
    fail "an empty folder was signed"
fi
if RELEASE_GPG_PRIVATE_KEY=$secret RELEASE_GPG_PASSPHRASE="correct horse" "$script" "$work/nonexistent" >/dev/null 2>&1; then
    fail "a missing folder was accepted"
fi

if [ "$failures" -eq 0 ]; then
    echo "all sign-release tests passed"
else
    echo "$failures failure(s)" >&2
    exit 1
fi
