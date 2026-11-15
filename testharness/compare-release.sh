#!/usr/bin/env bash
# Differential comparison of two dayzmake binaries (for example a released one and a fresh build).
#
#   testharness/compare-release.sh <old-dayzmake> <new-dayzmake> <dayz-dir>
#
# Runs the same commands with both binaries and compares stdout, stderr (paths normalised) and exit
# code, plus every file the commands write:
#   pbo     per <dayz-dir>/addons/*.pbo: `inspect`, and `unpack` into a work folder
#   config  `derapify` of every config.bin found in the unpacked PBOs
#   p3d     `inspect` of the first 50 .p3d found (sorted by PBO, then path)
#   golden  every command of tests/golden_cli.rs on tests/fixtures/golden/mod, plus img2paa on
#           1024x1024 gradient and noise PNGs (dxt1, dxt5)
#
# <dayz-dir> is only READ (a DayZ server or client install). All output goes to a work folder created
# with `mktemp -d /var/tmp/dayzmake-compare.XXXXXX`, removed at the end unless KEEP=1.
# Exit status: 0 when nothing differs, 1 when something differs, 2 on a usage or setup error.
set -euo pipefail

if [ "$#" -ne 3 ]; then
    echo "usage: $0 <old-dayzmake> <new-dayzmake> <dayz-dir>" >&2
    exit 2
fi
OLD_BIN=$(realpath -- "$1")
NEW_BIN=$(realpath -- "$2")
DAYZ=$(realpath -- "$3")
HERE=$(dirname -- "$(realpath -- "$0")")
REPO=$(dirname -- "$HERE")
GOLDEN="$REPO/tests/fixtures/golden/mod"
MODEL="$REPO/tests/fixtures/mlod/small.p3d"
SIGN="$REPO/tests/fixtures/bi/sign"
for f in "$OLD_BIN" "$NEW_BIN"; do
    [ -x "$f" ] || { echo "not executable: $f" >&2; exit 2; }
done
[ -d "$DAYZ/addons" ] || { echo "no addons folder in: $DAYZ" >&2; exit 2; }
[ -d "$GOLDEN" ] || { echo "missing golden fixture: $GOLDEN" >&2; exit 2; }

WORK=$(mktemp -d /var/tmp/dayzmake-compare.XXXXXX)
# The work folder never lies inside the DayZ install (mktemp puts it under /var/tmp, check anyway).
case "$WORK/" in "$DAYZ"/*) echo "work folder inside the DayZ install" >&2; exit 2 ;; esac
cleanup() {
    if [ "${KEEP:-0}" = 1 ]; then
        echo "kept work folder: $WORK"
    else
        rm -rf -- "$WORK"
    fi
}
trap cleanup EXIT

# shellcheck disable=SC2034  # read through indirect expansion in both()
BIN_old=$OLD_BIN
# shellcheck disable=SC2034
BIN_new=$NEW_BIN
mkdir -p "$WORK/old" "$WORK/new" "$WORK/diffs"

declare -A TOTAL=() SAME=() DIFFER=()
DIFFS=()
CATEGORIES=(pbo config p3d golden)

# run_side <side> <outprefix> <cwd> <stdin-file> <command...>: runs the command with a clean environment
# and stores <outprefix>.out, <outprefix>.err (paths normalised) and <outprefix>.code.
run_side() {
    local side=$1 prefix=$2 cwd=$3 input=$4
    shift 4
    local root="$WORK/$side" code=0
    mkdir -p "$root/home"
    env -u BIOUTPUT -u STEAM_ROOT -u DAYZ_TOOLS -u PROTON \
        HOME="$root/home" XDG_CONFIG_HOME="$root/home/config" XDG_CACHE_HOME="$root/home/cache" \
        XDG_DATA_HOME="$root/home/data" NO_COLOR=1 \
        bash -c 'cd "$1" && shift && exec "$@"' _ "$cwd" "$@" \
        <"$input" >"$prefix.out" 2>"$prefix.err" || code=$?
    echo "$code" >"$prefix.code"
    sed -i "s|$root|[SIDE]|g" "$prefix.out" "$prefix.err" 2>/dev/null || true
}

# record <category> <id> <differing-parts...>: counts one comparison and stores the differences.
record() {
    local cat=$1 id=$2
    shift 2
    TOTAL[$cat]=$(( ${TOTAL[$cat]:-0} + 1 ))
    if [ "$#" -eq 0 ]; then
        SAME[$cat]=$(( ${SAME[$cat]:-0} + 1 ))
    else
        DIFFER[$cat]=$(( ${DIFFER[$cat]:-0} + 1 ))
        DIFFS+=("$cat|$id|$*")
    fi
}

# tree_diff <old-tree> <new-tree> <report-file> <parts-array-name>: compares two folders file by file.
# A text file whose lines are the same in another order counts as "files(order-only)".
tree_diff() {
    local ta=$1 tb=$2 report=$3 rel real=0 order=0
    local -n out=$4
    : >"$report"
    if ! diff <(cd "$ta" && find . -type f | sort) <(cd "$tb" && find . -type f | sort) >>"$report"; then
        real=1
    fi
    while IFS= read -r rel; do
        [ -f "$tb/$rel" ] || continue
        cmp -s "$ta/$rel" "$tb/$rel" && continue
        if cmp -s <(sort "$ta/$rel") <(sort "$tb/$rel"); then
            order=1
            echo "order-only: $rel" >>"$report"
            { echo "  old:"; head -c 600 "$ta/$rel"; echo "  new:"; head -c 600 "$tb/$rel"; } >>"$report"
        else
            real=1
            echo "differs: $rel" >>"$report"
        fi
    done < <(cd "$ta" && find . -type f | sort)
    [ "$real" = 0 ] || out+=("files")
    [ "$order" = 0 ] || out+=("files(order-only)")
    [ -s "$report" ] || rm -f -- "$report"
}

# compare_pair <category> <id> <old-prefix> <new-prefix> [<old-tree> <new-tree>]
compare_pair() {
    local cat=$1 id=$2 a=$3 b=$4 ta=${5:-} tb=${6:-} parts=() n
    n=$(printf '%s' "$cat-$id" | tr -c 'A-Za-z0-9._-' '_')
    for part in out err code; do
        if ! cmp -s "$a.$part" "$b.$part"; then
            # Same lines in another order is reported as such (warning order, header extension order).
            if cmp -s <(sort "$a.$part") <(sort "$b.$part"); then
                parts+=("$part(order-only)")
            elif [ "$part" = err ] && cmp -s <(sort "$a.$part") <(grep -v 'were suppressed to prevent spam' "$b.$part" | sort); then
                parts+=("err(suppression-summary-only)")
            elif [ "$part" = err ] && cmp -s <(sort "$a.$part" | grep -vx '') <(grep -v 'were suppressed to prevent spam' "$b.$part" | sort | grep -vx ''); then
                parts+=("err(suppression-summary-only)")
            else
                parts+=("$part")
            fi
            { echo "--- old $part"; head -c 1500 "$a.$part"; echo "--- new $part"; head -c 1500 "$b.$part"; } \
                >>"$WORK/diffs/$n.$part.txt" 2>/dev/null || true
        fi
    done
    if [ -n "$ta" ]; then
        if [ -d "$ta" ] && [ -d "$tb" ]; then
            tree_diff "$ta" "$tb" "$WORK/diffs/$n.tree.txt" parts
        elif [ -d "$ta" ] || [ -d "$tb" ]; then
            parts+=("files(one side missing)")
        fi
    fi
    record "$cat" "$id" "${parts[@]}"
}

# both <category> <id> <stdin-file> <cwd-under-side-root> <tree-under-side-root-or-""> <command...>
# In the command, `{R}` stands for the side's work root and `{B}` for its binary.
both() {
    local cat=$1 id=$2 input=$3 cwdrel=$4 treerel=$5
    shift 5
    local n side args
    n=$(printf '%s' "$cat-$id" | tr -c 'A-Za-z0-9._-' '_')
    for side in old new; do
        local bin="BIN_$side"
        args=()
        for a in "$@"; do
            a=${a//\{R\}/$WORK/$side}
            args+=("${a//\{B\}/${!bin}}")
        done
        mkdir -p "$WORK/$side/$cwdrel"
        run_side "$side" "$WORK/$side/res-$n" "$WORK/$side/$cwdrel" "$input" "${args[@]}"
    done
    if [ -n "$treerel" ]; then
        compare_pair "$cat" "$id" "$WORK/old/res-$n" "$WORK/new/res-$n" "$WORK/old/$treerel" "$WORK/new/$treerel"
    else
        compare_pair "$cat" "$id" "$WORK/old/res-$n" "$WORK/new/res-$n"
    fi
}

# ---------------------------------------------------------------- addons: pbo, config, p3d
P3D_LEFT=50
shopt -s nullglob
PBOS=("$DAYZ"/addons/*.pbo)
shopt -u nullglob
[ "${#PBOS[@]}" -gt 0 ] || { echo "no .pbo files in $DAYZ/addons" >&2; exit 2; }
# The glob expands sorted (C locale ordering is not needed, both sides see the same list).
for pbo in "${PBOS[@]}"; do
    id=$(basename -- "$pbo")
    echo "== $id" >&2
    both pbo "inspect/$id" /dev/null . "" "{B}" inspect "$pbo"
    both pbo "unpack/$id" /dev/null . "unpack/$id" "{B}" unpack -f "$pbo" "{R}/unpack/$id"
    # derapify of every config.bin (list taken from the old tree; a different file set shows as a files difference)
    if [ -d "$WORK/old/unpack/$id" ]; then
        while IFS= read -r -d '' cfg; do
            rel=${cfg#"$WORK/old/unpack/$id/"}
            both config "$id/$rel" /dev/null . "" "{B}" derapify "{R}/unpack/$id/$rel"
        done < <(find "$WORK/old/unpack/$id" -type f -name config.bin -print0 | sort -z)
        if [ "$P3D_LEFT" -gt 0 ]; then
            while IFS= read -r -d '' m && [ "$P3D_LEFT" -gt 0 ]; do
                rel=${m#"$WORK/old/unpack/$id/"}
                both p3d "$id/$rel" /dev/null . "" "{B}" inspect "{R}/unpack/$id/$rel"
                P3D_LEFT=$((P3D_LEFT - 1))
            done < <(find "$WORK/old/unpack/$id" -type f -iname '*.p3d' -print0 | sort -z)
        fi
    fi
    rm -rf -- "$WORK/old/unpack" "$WORK/new/unpack"
done

# ---------------------------------------------------------------- golden CLI (tests/golden_cli.rs)
# A Python 3 helper writes the PNGs (no third-party modules, fixed seed).
mkpng() { python3 -I - "$@" <<'PY'
import random, struct, sys, zlib
kind, w, h, path = sys.argv[1], int(sys.argv[2]), int(sys.argv[3]), sys.argv[4]
rng = random.Random(1234)
rows = []
for y in range(h):
    row = bytearray([0])
    for x in range(w):
        if kind == "noise":
            px = (rng.randrange(256), rng.randrange(256), rng.randrange(256), 255)
        elif kind == "alpha":
            px = (x * 4 % 256, y * 4 % 256, 128, 255 - x * 2)
        else:
            px = (x * 255 // max(w - 1, 1), y * 255 // max(h - 1, 1), 128, 255)
        row += bytes(px)
    rows.append(bytes(row))
def chunk(t, d):
    c = struct.pack(">I", len(d)) + t + d
    return c + struct.pack(">I", zlib.crc32(t + d) & 0xFFFFFFFF)
data = b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", w, h, 8, 6, 0, 0, 0)) \
    + chunk(b"IDAT", zlib.compress(b"".join(rows), 9)) + chunk(b"IEND", b"")
open(path, "wb").write(data)
PY
}
mkdir -p "$WORK/png"
mkpng gradient 64 64 "$WORK/png/grad64.png"
mkpng alpha 64 64 "$WORK/png/alpha64.png"
mkpng gradient 1024 1024 "$WORK/png/grad1024.png"
mkpng noise 1024 1024 "$WORK/png/noise1024.png"

# golden <id> <script>: a fresh copy of the golden mod in <side>/g/<id>/mod (keys one level up), then
# runs the script with bash there. `$B` is the binary, `run ARGS...` runs it and prints `exit=N`.
golden() {
    local id=$1 script=$2 side
    for side in old new; do
        local g="$WORK/$side/g/$id"
        rm -rf -- "$g"
        mkdir -p "$g"
        cp -r -- "$GOLDEN" "$g/mod"
        cp -- "$MODEL" "$g/mod/data/model.p3d"
        cp -- "$SIGN/armk.biprivatekey" "$SIGN/armk.bikey" "$g/"
        cp -- "$WORK"/png/*.png "$g/"
    done
    both golden "$id" /dev/null "g/$id/mod" "g/$id" bash -c \
        'B=$1; run() { "$B" "$@"; echo "exit=$?"; }; eval "$2"' _ "{B}" "$script"
    # drop the png copies from the comparison (identical on both sides) to keep the trees small
}

golden help 'run --help'
golden version 'run --version'
golden preprocess_file 'run preprocess config.cpp out.cpp'
golden preprocess_stdin 'run preprocess -i . < config.cpp'
golden rapify_file 'run rapify config.cpp config.bin'
golden rapify_stdin 'run rapify -i . < config.cpp | sha256sum'
golden derapify_default 'run rapify config.cpp config.bin; run derapify config.bin out.cpp'
golden derapify_indent 'run rapify config.cpp config.bin; run derapify -d "	" config.bin out.cpp'
golden derapify_stdin 'run rapify config.cpp config.bin; run derapify < config.bin'
golden pack 'run pack . ../out.pbo'
golden pack_headerext 'run pack -e product=golden . ../out.pbo'
golden pack_headerext_multi 'run pack -e product=golden -e aaa=1 -e zzz=2 . ../out.pbo'
golden pack_exclude 'run pack -x "*.txt" . ../out.pbo'
golden build 'run build . ../out.pbo'
golden build_headerext_multi 'run build -e product=golden -e aaa=1 -e zzz=2 . ../out.pbo'
golden build_signed 'run build -k ../armk.biprivatekey . ../out.pbo'
golden build_signed_custom 'run build -k ../armk.biprivatekey . ../out.pbo; run build -k ../armk.biprivatekey -s ../sig.bisign -f . ../out2.pbo'
golden inspect_pbo_file 'run build . ../out.pbo; run inspect ../out.pbo'
golden inspect_pbo_stdin 'run build . ../out.pbo; run inspect < ../out.pbo'
golden inspect_pbo_multi_ext 'run pack -e product=golden -e aaa=1 -e zzz=2 . ../out.pbo; run inspect ../out.pbo'
golden inspect_p3d 'run inspect data/model.p3d'
golden unpack_tree 'run build . ../out.pbo; run unpack ../out.pbo ../x'
golden unpack_use_prefix 'run build . ../out.pbo; run unpack --use-prefix ../out.pbo ../x'
golden unpack_multi_ext 'run pack -e product=golden -e aaa=1 -e zzz=2 . ../out.pbo; run unpack ../out.pbo ../x'
golden unpack_existing 'run build . ../out.pbo; run unpack ../out.pbo ../x; run unpack ../out.pbo ../x; run unpack -f ../out.pbo ../x'
golden cat_stdout 'run build . ../out.pbo; run cat ../out.pbo "data\\readme.txt"'
golden cat_to_file 'run build . ../out.pbo; run cat ../out.pbo "data\\readme.txt" t'
golden cat_missing 'run build . ../out.pbo; run cat ../out.pbo "data\\nope.txt"'
# Random keys: only sizes and exit codes are compared.
golden keygen_sign_verify 'run build . ../out.pbo; run keygen k; run sign k.biprivatekey ../out.pbo; run verify k.bikey ../out.pbo; wc -c k.bikey k.biprivatekey ../out.pbo.k.bisign > sizes.txt; rm k.bikey k.biprivatekey ../out.pbo.k.bisign'
golden sign_v3 'run build . ../out.pbo; run sign ../armk.biprivatekey ../out.pbo'
golden sign_v2 'run build . ../out.pbo; run sign --v2 ../armk.biprivatekey ../out.pbo'
golden verify_ok 'run build . ../out.pbo; run sign ../armk.biprivatekey ../out.pbo; run verify ../armk.bikey ../out.pbo'
golden verify_tampered 'run build . ../out.pbo; run sign ../armk.biprivatekey ../out.pbo; python3 -I -c "
p = \"../out.pbo\"
d = bytearray(open(p, \"rb\").read())
i = d.index(b\"#define SCALE\")
d[i + 1] ^= 1
open(p, \"wb\").write(d)
"; run verify ../armk.bikey ../out.pbo'
golden img2paa_dxt1 'run img2paa -t dxt1 ../grad64.png out.paa'
golden img2paa_dxt5 'run img2paa -t dxt5 ../alpha64.png out.paa'
golden img2paa_rgba 'run img2paa -t rgba ../alpha64.png out.paa'
golden img2paa_default 'run img2paa ../alpha64.png out.paa'
golden img2paa_bad_type 'run img2paa -t bogus ../alpha64.png out.paa'
golden img2paa_compress 'run img2paa -c -t dxt5 ../alpha64.png out.paa'
golden img2paa_gradient_1024_dxt1 'run img2paa -t dxt1 ../grad1024.png out.paa'
golden img2paa_gradient_1024_dxt5 'run img2paa -t dxt5 ../grad1024.png out.paa'
golden img2paa_noise_1024_dxt1 'run img2paa -t dxt1 ../noise1024.png out.paa'
golden img2paa_noise_1024_dxt5 'run img2paa -t dxt5 ../noise1024.png out.paa'
golden paa2img 'run img2paa -t rgba ../alpha64.png out.paa; run paa2img out.paa back.png'
golden paa2img_1024 'run img2paa -t dxt5 ../noise1024.png out.paa; run paa2img out.paa back.png'
golden binarize_without_proton 'mkdir ../steam; STEAM_ROOT=$(realpath ../steam) run binarize data/model.p3d out.p3d'
golden existing_targets 'run rapify config.cpp config.bin; run img2paa ../alpha64.png g.paa; run build . ../out.pbo
for c in "rapify config.cpp t.bin" "derapify config.bin t.cpp" "preprocess config.cpp t.txt" "build . ../b.pbo" "pack . ../p.pbo" "keygen kk" "sign ../armk.biprivatekey ../out.pbo ../s.bisign" "img2paa ../alpha64.png t.paa" "paa2img g.paa t.png"; do run $c; run $c; done
rm -f kk.biprivatekey kk.bikey'
golden missing_inputs 'run rapify nope.cpp x; run inspect nope.pbo; run unpack nope.pbo d'
quota='mkdir ../q; cd ../q; printf "golden\\\\quota\n" > "\$PBOPREFIX\$"; for i in 00 01 02 03 04 05 06 07 08 09 10 11; do printf "\\0raP\\0\\0\\0\\0" > c$i.cpp; done; '
golden warning_quota "$quota"'run build . ../out.pbo'
golden warning_mute "$quota"'run build -w prerapified-file . ../out.pbo'
golden warning_verbose "$quota"'run build -v . ../out.pbo'
golden parse_error 'printf "class A { = 1; };\n" > bad.cpp; run rapify bad.cpp bad.bin'
golden empty_value 'printf "class A { x = ; };\n" > bad.cpp; run rapify bad.cpp bad.bin; run derapify bad.bin'

# ---------------------------------------------------------------- summary
echo
echo "== summary (old: $OLD_BIN, new: $NEW_BIN, dayz: $DAYZ)"
all_same=0 all_diff=0
for cat in "${CATEGORIES[@]}"; do
    t=${TOTAL[$cat]:-0} s=${SAME[$cat]:-0} d=${DIFFER[$cat]:-0}
    printf '%-7s %5d compared, %d identical, %d different\n' "$cat" "$t" "$s" "$d"
    all_same=$((all_same + s))
    all_diff=$((all_diff + d))
done
echo "total   $((all_same + all_diff)) compared: $all_same identical, $all_diff different"
if [ "$all_diff" -gt 0 ]; then
    echo
    echo "== differences (category | id | parts that differ); one excerpt per category and kind of difference"
    declare -A SHOWN=()
    for line in "${DIFFS[@]}"; do
        echo "$line"
        IFS='|' read -r cat id parts <<<"$line"
        [ -z "${SHOWN[$cat|$parts]:-}" ] || continue
        SHOWN[$cat|$parts]=1
        n=$(printf '%s' "$cat-$id" | tr -c 'A-Za-z0-9._-' '_')
        for f in "$WORK/diffs/$n".*.txt; do
            [ -e "$f" ] || continue
            echo "  [$(basename -- "$f")]"
            head -n 20 -- "$f" | sed 's/^/    /'
        done
    done
    exit 1
fi
