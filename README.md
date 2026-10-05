# Confetti

A cozy editor for `unrealircd.conf`. Nested blocks are drawn as nested trays you
can fold, with the real file live alongside. Local files now; SSH later.

The full design is in [design.html](design.html) — open it in a browser.

## Status

Milestones 1 and 2 of 6. What works today:

**Documentation packs.** Confetti reads the directive tree out of UnrealIRCd's
own `src/conf.c` (plus 46 modules) and the prose out of the wiki archive that
ships beside it, then joins the two on the `block::item` path both already use.
Both halves are pinned to one release, so hints describe *your* server rather
than the newest one. 167 of the 174 entries in the shipped `example.conf` come
back with an explanation. Packs are cached per version, so it is built once.

**Typed matchers.** A `mask { }` is not a text box. It has three shapes — one
host, a list of hosts, or named criteria — and each renders as itself, with the
criterion vocabulary as a picker and yes/no criteria as toggles.

**The Console layout.** File tree, one dense line per block grouped by category
with a colour rail, and an inspector. Point it at an install or at a `conf/` and
it finds every `.conf` underneath either way, including subfolders, with block
counts and include arrows. `Ctrl K` filters.

**Two themes, one renderer.** Cream and Console are a token swap — density and
palette are separate axes, so the layout is identical in both.


- **Lossless parser.** Reads a config into a CST where every byte is accounted
  for. Comments, blank lines, tabs, `#` vs `//`, preprocessor directives and
  stray tokens are all nodes.
- **Round-trip guarantee.** An unedited document renders byte for byte. Proven
  against all 33 configuration files UnrealIRCd ships — 748,656 bytes, 13,152
  entries, zero parse errors.
- **The tray workspace.** Five tiers of nesting, colour-coded by category, with
  plain-English names and the real directive name underneath.
- **Editing.** Click a value, change it, save. Only those bytes are rewritten;
  a `.conf~` backup is kept.

Not yet: SSH and SFTP, the docs packs, `configtest` / `rehash`, the quest line,
value types beyond plain text.

## Running it

```sh
npm install
npm run tauri dev
```

Needs Rust and, on Windows, the MSVC toolchain and WebView2.

## Layout

```
src/                  frontend — plain HTML/CSS/JS, no bundler
  index.html          the three panes
  styles.css          the cozy chrome and the five-tier depth ramp
  main.js             tray rendering, plain-name dictionary, editing
src-tauri/
  src/cst.rs          the lossless parser
  src/lib.rs          commands: open, edit, save
  tests/corpus.rs     every shipped config must round-trip
design.html           the design study this is being built from
```

## The parser

Modelled on UnrealIRCd's own tokenizer in `src/conf.c`
(`config_parse_with_offset`), including the quirks that matter:

- An entry carries at most two words. A third is "Extra data detected".
- `}` and `};` both close a block — conf.c simulates the missing `;`.
- `=` is whitespace.
- Bare words break on space, tab, newline and `;` — **not** on `{`, so
  `oper bob{` really does yield the value `bob{`. We reproduce that and warn.
- `'single quotes'` mark an entry literal; `"double"` allows `\"` escapes.
- `#`, `//` and `/* */` comments; `/* */` does not nest.
- `@if` / `@else` / `@endif` / `@define` run to end of line.

Two invariants hold it together:

- `verify_tiling()` — the children of every node tile that node's span exactly,
  and the document tiles the whole source. Nothing was skipped.
- `render()` — the original text plus a sorted edit list. Untouched regions are
  emitted exactly as read, so losslessness is structural rather than hoped for.

## Tests

```sh
cd src-tauri && cargo test
```

`corpus.rs` points at `C:\Users\Valerie\unrealircd-pr\doc\conf` and skips itself
if that isn't there.

## What the council changed

Three Opus reviewers went over the plan and the code — an UnrealIRCd domain
expert, a systems architect, and a cognitive-load specialist. They disagreed
usefully, and between them they found **five data-corruption bugs in the first
merge/split implementation**, all of which produced output that parsed cleanly
and passed `verify_tiling()`:

1. Merging into a one-line block (`set { x; }`) put the moved directives
   *before* the block, at the top level.
2. `with_leading_comments` tested `line.ends_with("*/")`, so
   `class clients { } /* the usual */` above a `set` block swallowed the whole
   class into it.
3. Merging across an `@if` moved settings into or out of a conditional.
4. CRLF files came back with mixed line endings.
5. Splitting a directive that shared a line with `}` took the brace with it,
   nesting the new block inside the old one (`set::set::maxchannelsperuser`).

Each now has a test named after it. Two deeper corrections came out of reading
the server rather than the docs:

- **`render()` silently dropped overlapping edits.** `Doc::edit` now returns a
  `Result` and refuses anything it cannot apply faithfully — overlapping spans,
  spans outside the text, spans that would split a character.
- **The duplicate model was backwards.** `CheckDuplicate` (conf.c:7299) *warns*;
  `_conf_set` applies the second copy anyway, so **the last one wins**. And
  `set::anti-flood` and `set::options` are *cumulative* while
  `set::tls::certificate` builds a list — so commenting a duplicate out can
  delete live settings. Confetti now refuses to auto-resolve anything that is
  not a plain scalar, and never treats a repeated `certificate` as a conflict.

Merging is offered **only for `set` blocks**, because for the others it changes
behaviour: `allow` and `deny channel` are matched in *reverse* config order
(`add_ListItem` prepends, list.c:478), `class` blocks already merge by name but
reset `options` and `connfreq` each time, `security-group` merges additively
onto the built-ins, and duplicate `oper` names are legal with the later one
winning. Each refusal says which of those applies.

## Checking a configuration

`analyse.rs` reads the whole folder and reports what `configtest` cannot: an
`oper` pointing at a class that does not exist, two listen blocks fighting over
a port, values still set to what the example ships with.

One rule governs it: **only compare files the server loads together.** A conf
directory holds alternatives, not companions — five translations of
example.conf, an optional modules list, a spamfilter.conf whose include is
commented out. Cross-checking those invented 33 errors that did not exist. The
reachable set is computed from the include graph instead.

Severities follow what the server actually does, which is often not "error":

- A dangling `class` reference does not stop the server. It prints a
  `config_status` notice that scrolls past and substitutes the built-in default
  class — **100 clients, 8000-byte sendq**. The finding says that, because what
  happens is more alarming than the word "error".
- Two `class` blocks with one name is a *note*: the server merges them.
- Two `oper` blocks with one name is a *warning*: legal, later one wins.
- `ip *` binds both IPv4 and IPv6, and port ranges expand, so both are
  accounted for before declaring a clash.

Findings are dismissible per folder, and notes are never counted in the badge —
a number that can never reach zero is just background anxiety.

## Undo

Snapshots, not inverse edits, and at workspace level so a change touching two
files unwinds as one. `Ctrl+Z` / `Ctrl+Shift+Z`, 200 deep, and the button says
what it would undo. Every destructive operation shows a diff first and writes
nothing until you approve it.
