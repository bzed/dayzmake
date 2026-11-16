# HTML documentation on GitHub Pages and signed release checksums: implementation plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Publish the dayzmake documentation as an mdBook site with its own theme on
`https://bzed.github.io/dayzmake/`, and attach GPG-signed `SHA256SUMS`/`SHA512SUMS` files to every release.

**Architecture:** An mdBook in `docs/book/` whose chapters pull their text out of `README.md` through
`{{#include}}` anchors, so the README stays the single source. A test in `tests/book_sources.rs` checks that
the anchors, the summary and the README headings stay in sync. A new workflow `pages.yml` builds the book plus
the `dayzmake-core` rustdoc and deploys it on `main`. `release.yml` computes checksums over the release
archives and signs them with a key held in repository secrets, through `scripts/sign-release.sh`.

**Tech Stack:** mdBook 0.5.4 (build tool only, not a crate dependency), GitHub Actions (`actions/configure-pages`,
`actions/upload-pages-artifact`, `actions/deploy-pages`), GnuPG, self-hosted WOFF2 fonts (SIL OFL).

**Spec:** the user's request of 2026-11-16 (HTML docs on GitHub Pages with a designed stylesheet; a GPG key in a
temp dir, stored as a GitHub secret, used to sign SHA-256/512 files of the built binaries) plus the design
tokens below.

## Already done by the controller (do not redo)

- Branch `feat/pages-and-signed-checksums` exists.
- GitHub Pages is enabled for `bzed/dayzmake` with `build_type: workflow`.
- Signing key generated (ed25519, sign-only, expires 2029-11-15), passphrase-protected:
  - fingerprint `6624 41B3 6A57 85AD 18FB  8667 DF75 3D99 D064 EB7E`
  - UID `dayzmake release signing key (https://github.com/bzed/dayzmake)`
  - repository secrets `RELEASE_GPG_PRIVATE_KEY` (ASCII-armoured secret key) and `RELEASE_GPG_PASSPHRASE`.
- The **public** key is at
  `/tmp/claude-1000/-home-bzed-workspace-dayz-tools-dayzmake/b1f9ad19-b34a-4777-a7af-43ec9e54c3bc/scratchpad/release-key.asc`.
  Implementers never see or touch the private key. For local tests they make their own throwaway key under a
  `mktemp -d` directory.

## Global Constraints

- AGENTS.md applies in full: temp/build folders under `/tmp` (never `$HOME`), conventional commit prefixes,
  one logical change per commit, `make check` passes at the end, README and `USAGE` in `src/run.rs` stay in sync.
- GitHub Actions are pinned by full commit SHA with the version in a trailing comment, like the existing
  workflows (`uses: actions/checkout@3d3c42e5... # v7.0.1`). Look up current release SHAs with
  `gh api repos/<owner>/<repo>/git/ref/tags/<tag>` (dereference annotated tags).
- Workflows use least privilege: top-level `permissions: contents: read`, wider permissions only on the job that
  needs them.
- No third-party requests from the published site: fonts are self-hosted in `docs/book/theme/fonts/`, no Google
  Fonts link, no analytics, no CDN scripts.
- Fonts must be SIL OFL 1.1; their licence files go next to them (`docs/book/theme/fonts/OFL-*.txt`).
- `docs/book/` must not enter the crate package (root `Cargo.toml` already has an `include` list; verify with
  `cargo package --list -p dayzmake`).
- mdBook output goes to `$(CARGO_TARGET_DIR)/book` locally (the Makefile's target dir is under `/tmp`), never into
  the source tree; `docs/book/book/` is in `.gitignore` anyway.
- No secret, passphrase or private key material in the repo, logs or command lines (`ps`-visible). Pass the
  passphrase to gpg by file descriptor or file, never as an argument.

## Design tokens (from the frontend-design pass)

Subject: reference manual for a DayZ modding CLI. Audience: modders who pack, sign and binarize mods, often on
Linux. Primary job: find the command and its flags fast, then read the details.

Direction: a **field manual for the supply depot**. DayZ loot crates and ammo boxes carry stencilled markings;
PBOs are the crates of a mod. Chapter titles are set in a stencil face; everything else is quiet, legible
manual text on a cool, slightly green paper.

| Token | Light | Dark | Use |
|---|---|---|---|
| `--paper` | `#E8ECE6` | `#18201C` | page background |
| `--ink` | `#1E2B24` | `#D5DDD2` | body text |
| `--olive` | `#4F6131` | `#A7C06A` | links, sidebar active item |
| `--slate` | `#5E6B66` | `#8E9C95` | secondary text, sidebar, borders |
| `--plate` | `#D9DFD6` | `#212B26` | code blocks, sidebar background |
| `--marker` | `#E2B007` | `#E2B007` | the one accent: 4 px left bar of the active sidebar item, `:focus-visible` ring, `mark` |

Type:
- Chapter titles (`h1` only): **Big Shoulders Stencil Display**, weight 800, `font-size: clamp(2.4rem, 6vw, 3.6rem)`,
  line-height 1, letter-spacing `0.01em`, sentence case as written. This is the single bold element.
- `h2`–`h4` and body: **Atkinson Hyperlegible Next** (variable or 400/700 + italic 400). Body 1.0625rem /
  line-height 1.6; scale `h2` 1.6rem, `h3` 1.25rem, `h4` 1.0625rem bold; headings weight 700, no all caps.
- Code: **JetBrains Mono** 400/700, 0.92em. Code only; no monospace for labels.
- Measure: `.content main { max-width: 72ch; }`; code blocks may scroll horizontally inside themselves, the page
  never does (check at 360 px width).

Layout: mdBook's own frame (sidebar left, content left-aligned). Sidebar on `--plate` with `--slate` text; the
active chapter gets the `--marker` bar and `--ink` text. CLI synopsis blocks (the `Usage:` block in the CLI
chapter) look like a stamped label plate: `--plate` background, 2 px `--slate` border, no shadow, radius 2px.
Ordinary code blocks: `--plate` background, no border, radius 2px. Tables: header row bold on `--plate`,
1 px `--slate` row rules.

Rules: no decorative motion (only mdBook's own sidebar toggle), honour `prefers-reduced-motion`; no gradients,
no shadows, no eyebrow labels, no `→` in links, no middle-dot meta strings. Visible `:focus-visible` outline
(2 px `--marker`, offset 2 px). Contrast of `--ink`/`--paper`, `--olive`/`--paper` and `--slate`/`--plate` must be
≥ 4.5:1 in both modes (check with any WCAG contrast calculator and note the ratios in the commit message).

Themes: set `default-theme = "light"` and `preferred-dark-theme = "coal"` in `book.toml`; the stylesheet maps the
tokens onto mdBook's CSS variables under `.light` and `.coal`, and maps `.rust`, `.navy`, `.ayu` to the light or
dark tokens (`.rust` → light, `.navy`/`.ayu` → dark) so every theme in the picker looks like the design.
Syntax highlighting: keep mdBook's highlight.js but override the token colours to stay within the palette
(keywords `--olive`, strings `--ink` italic, comments `--slate`).

## Review Focus

1. A README section added later without a book chapter: the test must fail and name the heading.
2. An `{{#include}}` that names a missing anchor: mdBook only warns, so the test must fail and name the file and anchor.
3. A relative link in an included README part (`[x](testharness/README.md)`) is a 404 on the site: the test must fail and name the link; README uses absolute `https://github.com/bzed/dayzmake/blob/main/...` URLs for repo files.
4. A release run where the signing secrets are missing or the key is expired: the job must fail, not publish unsigned checksums.
5. Checksum files listing themselves or the `.asc` files, or paths with a `dist/` prefix: `sha256sum -c` run inside the download folder must pass.

## Amendment (2026-11-16): real reference documentation, not only the README

The user asked for reference documentation that is better than the README. Task 1 stays as written for the
pages that really are the README (introduction + history, building, testing, changes). **Task 1b** (after Task 1,
before Task 2) replaces the README-include chapters for usage, API index, MCP, models and library with
hand-written guides and a reference section. README stays the short overview; the book is the full manual.

Book structure after Task 1b (`SUMMARY.md`, mdBook part titles with `# ` lines):

- Introduction (README include) · Building from source (README include) · Verifying downloads (Task 4)
- **Guides** (task-oriented, each a worked example from an empty folder to a result):
  `guides/first-mod.md` (folder layout, `$PBOPREFIX$`, `config.cpp`, `build`, `keygen`, `sign`/`-k`, `verify`,
  the Workshop `@mod/addons`+`keys` layout), `guides/configs.md` (preprocess, includes and `-i`, rapify/derapify),
  `guides/binarize-linux.md` (DayZ Tools under Proton, `--proton-binarize`, the environment variables),
  `guides/textures.md` (img2paa/paa2img, types, compression, checking model textures with `inspect`),
  `guides/script-api.md` (the `api` commands, mods, cache, `--json` with `jq` examples),
  `guides/mcp.md` (registering `dayzmake mcp` in Claude Code and other MCP clients, example prompts)
- **Reference**:
  `reference/cli.md` (global synopsis, common options, exit status, stdin/stdout use of `-`/omitted paths if
  supported) and one page per command group: `reference/config.md` (rapify, preprocess, derapify),
  `reference/binarize.md`, `reference/pbo.md` (build, pack, inspect, unpack, cat), `reference/signing.md`
  (keygen, sign, verify; key/signature file formats and v2/v3), `reference/paa.md` (paa2img, img2paa),
  `reference/api.md` (every `api` subcommand), `reference/mcp.md` (the `mcp` command and **every MCP tool**:
  purpose, input schema as a table, result shape, an example request/response pair);
  `reference/environment.md` (every environment variable and path dayzmake reads: `BIOUTPUT`, `STEAM_ROOT`,
  `HOME`, `DAYZ_TOOLS`, `PROTON`, the API index cache location, Steam library discovery, Windows registry use);
  `reference/warnings.md` (every warning name usable with `-w`, what triggers it, example);
  `reference/header-extensions.md` (`-e key=value`, `product=dayz`, prefix handling, exclude globs and defaults);
  `reference/library.md` (README library include + links to `core/dayzmake_core/index.html` and docs.rs)
- Testing (README include) · Changes (README include)

Each command section (`## dayzmake <cmd>` / `## dayzmake api <sub>`) has, in this order: one-sentence purpose,
the synopsis line copied from `USAGE` in a `text` fence wrapped in `<div class="synopsis">`, an arguments table,
an options table (every option the synopsis line names, with its default), *Behaviour* (what is read, what is
written, overwrite rules, validation that can reject input), *Examples* (at least one, real output), and
*See also* links. Facts come from reading `src/run.rs`, `src/commands/`, `src/mcp.rs`, `src/api/` and
`crates/dayzmake-core/src/`, never from guessing; every example command is run on synthetic input
(`tests/fixtures/` or files you create) in a `mktemp -d` folder and its real output pasted (shorten long output
with an explicit `…` line).

Tests (in `tests/book_sources.rs`, written first, red then green):
- `every_command_has_a_reference_section`: for every `dayzmake <cmd>` / `dayzmake api <sub>` usage line in
  `dayzmake::run::USAGE`, some `docs/book/src/reference/*.md` has the heading `## dayzmake <cmd>` (resp.
  `## dayzmake api <sub>`). Message names the missing command.
- `every_option_is_documented_with_its_command`: every `-x`/`--long` option on a usage line appears (as
  `` `--long` `` or `` `-x` ``) inside that command's section. Message names command and option.
- `every_mcp_tool_is_documented`: every tool name the MCP server lists (use the public function in `src/mcp.rs`
  that builds the `tools/list` result, or drive `tools/list` through the same helper `tests/mcp.rs` uses) has the
  heading `### <tool_name>` in `reference/mcp.md`.
- `every_warning_is_documented`: every warning name defined in `dayzmake-core` (find the canonical list; if
  there is none, read the names from the source in the test and say so in a comment) appears as a heading in
  `reference/warnings.md`.
- Replace `every_readme_section_is_in_the_book` with `every_readme_section_is_in_the_book_or_mapped`: each README
  `## ` heading is either included through an anchor or listed in a `const README_COVERED_BY: &[(&str, &str)]`
  table (heading → book page) in the test, and that page exists.
- Keep `summary_entries_exist`, `includes_resolve`, `included_readme_links_are_absolute`.

Commit: `docs: guides and command reference for the HTML documentation`.
Review Focus addition: 6. A new CLI option or MCP tool added later without docs must fail `make test`.

---

### Task 1: Book skeleton, README anchors and the consistency test

**Files:**
- Create: `docs/book/book.toml`, `docs/book/src/SUMMARY.md`, one chapter file per README part under `docs/book/src/`
- Create: `tests/book_sources.rs`
- Modify: `README.md` (anchor comments, line 181 link to absolute URL), `.gitignore` (`docs/book/book/`)

**Interfaces:**
- Produces: anchor names in `README.md` of the form `<!-- ANCHOR: <name> -->` / `<!-- ANCHOR_END: <name> -->`,
  chapter files that Task 2 (theme) and Task 3 (workflow) build. `book.toml` `[output.html]` keys
  `additional-css = ["theme/dayzmake.css"]`, `default-theme`, `preferred-dark-theme`,
  `git-repository-url = "https://github.com/bzed/dayzmake"`, `site-url = "/dayzmake/"`.

Chapters (in this order in `SUMMARY.md`; the first is the landing page):
1. `introduction.md` – README intro paragraph + *History* (+ the status line). No badges (they load third-party
   images): the anchor starts after the badge lines.
2. `building.md` – *Building*.
3. `usage.md` – *Usage* (the CLI reference, including the `Usage:` synopsis block; give that block's fence the
   language `text` and wrap the include in `<div class="synopsis">` … `</div>` with blank lines around the
   include so Task 2 can style it).
4. `api-index.md` – *Script API index*.
5. `mcp.md` – *MCP server*.
6. `models.md` – *Inspecting models and checking their textures* and *Binarizing models on Linux*.
7. `library.md` – *Using dayzmake-core as a library*, plus one line linking to `core/dayzmake_core/index.html`
   (the rustdoc Task 3 publishes) and to docs.rs.
8. `testing.md` – *Testing*.
9. `verifying.md` – placeholder heading only in this task; Task 4 fills it.
10. `changes.md` – *Changes* (the version history, last because it is long).

Each chapter file is one `# <Title>` line followed by `{{#include ../../../README.md:<anchor>}}`. The included
README text keeps its own `##` heading out of the anchor (the chapter's `#` replaces it), so its `###`
subheadings render one level too deep; acceptable, do not rewrite README headings.

- [ ] **Step 1: Write the failing tests in `tests/book_sources.rs`** (std only, no new dependency; paths from
  `env!("CARGO_MANIFEST_DIR")`):
  - `summary_entries_exist`: every `](file.md)` in `docs/book/src/SUMMARY.md` exists under `docs/book/src/`.
  - `includes_resolve`: for every `{{#include <path>:<anchor>}}` in `docs/book/src/*.md`, the target file exists
    (resolved relative to the chapter) and contains exactly one `ANCHOR: <anchor>` line and one
    `ANCHOR_END: <anchor>` line after it. Failure message names chapter, path and anchor.
  - `every_readme_section_is_in_the_book`: every `## ` heading of `README.md` lies inside some anchor range that
    a chapter includes. Failure message names the heading.
  - `included_readme_links_are_absolute`: inside included anchor ranges, every Markdown link target `](x)` is
    `http://`, `https://`, `mailto:` or starts with `#`. Failure message names the link.
- [ ] **Step 2: Run** `cargo test --locked --test book_sources` – expected FAIL (no `docs/book` yet).
- [ ] **Step 3: Create `book.toml`, `SUMMARY.md`, the chapters, the README anchors, fix README line 181**
  (`https://github.com/bzed/dayzmake/blob/main/testharness/README.md`), add `docs/book/book/` to `.gitignore`.
- [ ] **Step 4: Run** `cargo test --locked --test book_sources` – PASS; then
  `mdbook build docs/book -d "$(mktemp -d /tmp/book.XXXXXX)"` – exits 0 with no `WARN`.
- [ ] **Step 5: Commit** `docs: mdBook sources that include the README sections`.

### Task 2: Theme (stylesheet and fonts)

**Files:**
- Create: `docs/book/theme/dayzmake.css`, `docs/book/theme/fonts/*.woff2`, `docs/book/theme/fonts/OFL-*.txt`
- Modify: `docs/book/book.toml` (only if a key is missing)

**Interfaces:**
- Consumes: `book.toml` `additional-css` from Task 1; the `.synopsis` wrapper in `usage.md`.

- [ ] **Step 1: Fetch the fonts** (Latin subset WOFF2) from the `@fontsource` npm packages via
  `https://cdn.jsdelivr.net/npm/@fontsource/<family>@<exact version>/files/…` into a `mktemp -d` dir, check the
  package licence is OFL-1.1, copy the WOFF2 files and the OFL text into `docs/book/theme/fonts/`. Families:
  `big-shoulders-stencil-display` (800), `atkinson-hyperlegible-next` (400, 700, 400 italic), `jetbrains-mono`
  (400, 700). Keep the total under 400 KiB.
- [ ] **Step 2: Write `dayzmake.css`** implementing the design tokens section exactly: `@font-face` with
  `font-display: swap` and relative `url(fonts/…)`, token variables per theme class mapped onto mdBook's
  variables (`--bg`, `--fg`, `--sidebar-bg`, `--sidebar-fg`, `--sidebar-active`, `--links`, `--inline-code-color`,
  `--quote-bg`, `--table-header-bg`, `--table-border-color`, `--searchbar-bg`, …; read the mdBook 0.5.4
  `css/variables.css` shipped in a built book to get the full list), typography, `.synopsis pre`, focus ring,
  reduced motion.
- [ ] **Step 3: Verify visually.** Build into a `mktemp -d` dir, serve it with `python3 -m http.server -d <dir>`
  on a free port and take screenshots at 1280 px and 360 px wide in light and coal theme (headless Chromium
  `--screenshot --window-size=…` or Playwright if available). Check: stencil `h1`, no horizontal page scroll at
  360 px, sidebar active marker, synopsis plate, readable tables. Check `grep -r 'fonts.googleapis\|http' theme/`
  finds no external URL in the CSS. Record the contrast ratios.
- [ ] **Step 4: Commit** `docs: field-manual theme for the HTML documentation` (body: font licences, contrast
  ratios).

### Task 3: Pages workflow and Makefile target

**Files:**
- Create: `.github/workflows/pages.yml`
- Modify: `Makefile` (target `book`, added to `check` after `docsrs`), `AGENTS.md` (row `make book` in the
  *Before pushing* table and `docs/book/` in the layout table)

**Interfaces:**
- Consumes: `docs/book/` from Tasks 1–2.
- Produces: site layout `/` (book), `/core/` (rustdoc: `cargo doc -p dayzmake-core --all-features --no-deps
  --locked` output `target/doc/*` copied to `<book>/core/`), `/release-key.asc` (Task 4 adds the file to
  `docs/book/src/`, mdBook copies non-Markdown files from `src/`).

- [ ] **Step 1: Makefile `book` target**: if `mdbook` is on `PATH`, `mdbook build docs/book -d
  $(CARGO_TARGET_DIR)/book`; else `echo "skipping book: mdbook missing"` (same pattern as `docsrs`). Add to `check`.
- [ ] **Step 2: `pages.yml`**: triggers `push` to `main`, `pull_request`, `workflow_dispatch`. Job `build`
  (ubuntu-latest, `contents: read`): checkout, install `mdbook@0.5.4` with the pinned `taiki-e/install-action`
  already used in `ci.yml`, `cargo test --locked --test book_sources`, `mdbook build docs/book -d _site`, rustdoc
  into `_site/core`, `actions/upload-pages-artifact` with `path: _site`. Job `deploy` (`needs: build`,
  `if: github.event_name != 'pull_request' && github.ref == 'refs/heads/main'`, `permissions: pages: write,
  id-token: write`, `environment: github-pages` with `url: ${{ steps.deployment.outputs.page_url }}`,
  `concurrency: group: pages, cancel-in-progress: false`): `actions/deploy-pages`. All actions SHA-pinned.
- [ ] **Step 3: Verify**: `make book` succeeds; `actionlint .github/workflows/pages.yml` if installed (otherwise
  `python3 -c 'import yaml,sys; yaml.safe_load(open(sys.argv[1]))' .github/workflows/pages.yml`); `cargo package
  --list -p dayzmake | grep -c docs/book` prints `0`.
- [ ] **Step 4: Commit** `ci: build the documentation and deploy it to GitHub Pages`.

### Task 4: Signed checksums in the release workflow

**Files:**
- Create: `scripts/sign-release.sh`, `docs/book/src/release-key.asc` (copy of the public key above)
- Modify: `.github/workflows/release.yml`, `README.md` (new section `## Verifying downloads` with anchor,
  placed after *Building*), `docs/book/src/verifying.md` (include that anchor), `AGENTS.md` (*Releasing*: the key,
  its fingerprint, expiry 2029-11-15, the two secret names, how to extend the expiry and update the secret)

**Interfaces:**
- `scripts/sign-release.sh <dist-dir>`: reads env `RELEASE_GPG_PRIVATE_KEY` and `RELEASE_GPG_PASSPHRASE`;
  creates in `<dist-dir>`: `SHA256SUMS`, `SHA512SUMS` (bare file names, sorted, covering every regular file
  present before the script ran) and detached ASCII signatures `SHA256SUMS.asc`, `SHA512SUMS.asc`; exit non-zero
  with a message naming the missing variable when a secret is empty, and when gpg fails (expired key, wrong
  passphrase). `set -euo pipefail`, its own `GNUPGHOME=$(mktemp -d)` removed on exit, passphrase via
  `--pinentry-mode loopback --passphrase-fd` from a here-string or a 0600 temp file, never on the command line.
  After signing it runs `gpg --verify` on both signatures and `sha256sum -c`/`sha512sum -c` as a self-check.

- [ ] **Step 1: Write the test first**: `scripts/test-sign-release.sh` (no CI job needed, run by hand and noted in
  the commit) that makes a throwaway key in `mktemp -d`, a fake dist with two files, and asserts: the script
  succeeds; both sums files list exactly the two files; `gpg --verify` passes; with
  `RELEASE_GPG_PASSPHRASE=` the script fails and names the variable; with a wrong passphrase it fails. Run it –
  expected FAIL (script missing).
- [ ] **Step 2: Write `scripts/sign-release.sh`**, run the test – PASS. Run `shellcheck` on both scripts if
  installed.
- [ ] **Step 3: `release.yml`**: in `publish`, add `actions/checkout` (for the script), then after the download
  run `scripts/sign-release.sh dist` with the two secrets in `env:`, copy `docs/book/src/release-key.asc` to
  `dist/dayzmake-release-key.asc` **after** signing (so it is not in the sums), and keep `gh release create
  … dist/*`.
- [ ] **Step 4: Docs**: README *Verifying downloads* – fingerprint, where to get the key (the release asset and
  `https://bzed.github.io/dayzmake/release-key.asc`), and the commands
  `gpg --import dayzmake-release-key.asc`, `gpg --verify SHA256SUMS.asc SHA256SUMS`,
  `sha256sum -c --ignore-missing SHA256SUMS` (and the Windows equivalent `Get-FileHash -Algorithm SHA256`).
  Mention it in the 3.0.x/unreleased part of *Changes* if one exists, otherwise add an `### Unreleased` entry.
  `cargo test --locked --test book_sources` and `make book` pass.
- [ ] **Step 5: Commit** `ci: publish GPG-signed SHA-256 and SHA-512 checksums with each release`.

### Task 5: Final check

- [ ] `make check` passes (record the output tail).
- [ ] Controller: whole-branch review (Opus), fixes, push branch, open PR. Do not merge.
