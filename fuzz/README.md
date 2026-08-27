# denise-fuzz

Fuzz targets, outside the workspace on purpose: they need nightly and a
sanitiser, are never published, and the workspace asserts things about its
members that are not true of this crate. See `exclude` in the root manifest.

```sh
cd fuzz
cargo +nightly fuzz run <target> -- -rss_limit_mb=4096
cargo +nightly fuzz run <target> <artifact>   # replay one finding
```

CI runs every target for a minute per push — enough to catch regressions; the
long runs belong here, on somebody's machine, overnight. The form targets run
without `-timeout`: `kdl` parses some malformed files in exponential time, which
is upstream and unbounded, and a gate that goes red on a bug nobody here can fix
only teaches people to ignore red.

## Targets

| target | what goes in | what must hold |
|---|---|---|
| `decode_image` | arbitrary bytes, through `denise_image::decode` | no panic; a decoded picture agrees with its own size |
| `abi_session` | a *sequence* of C ABI calls with hostile arguments | no panic past `guard`; stale and out-of-range ids are refused, not chased |
| `parse_form` | arbitrary bytes, through `Form::parse` | no panic; errors have a position and print; nothing over `MAX_SOURCE` parses; **what parses round-trips byte-for-byte through `text()`** |
| | | libFuzzer's own `slow-unit-*` artifacts are read here too, by a person rather than by CI — see below |
| `build_form` | generated form-shaped trees — real widget kinds, colliding property names, plausible and nonsense values — through `Form::build` on a headless `Ui` | no panic in the builder or the widgets' `set`s; every id in `Built` exists in the tree; every collection item `items` reports, `item_path` can address |

`parse_form` is seeded with the repository's own `.dform` files *and* the
awkward corpus, so the fuzzer starts from text that reaches deep into the
parser rather than spending its budget rediscovering that `f` is not a form —
and it starts from the files that are already deliberate messes, which is the
neighbourhood both findings below came out of. `build_form` generates structure
rather than text — almost no arbitrary bytes parse, so a text fuzzer barely
reaches the builder at all; the generated trees are printed to source and fed
through `Form::parse` first, which keeps everything it finds reachable from a
real file.

The depth and size limits (`MAX_DEPTH`, `MAX_SOURCE`, `MAX_COMMENTED_DEPTH`)
are asserted from both sides: unit tests check that exceeding them is an error,
and `parse_form` checks that nothing past them succeeds.

## What they have found

`parse_form` earned its place in its first hour, twice. kdl eats whatever
stands between a closing brace and the next node: first the trailing
whitespace — `}` followed by two spaces comes back as `}`, the spaces *and*
the newline gone, gluing the next node onto the brace's line — and then, once
that was fixed, a whole `// note` written on the brace's line, and a `/* */`
comment with it. The first is invisible in any editor; the second is something
a person writes on purpose. Either would have surfaced as the designer
silently changing bytes on the first save of a hand-written file.

The repair is one rule for all of them, in `restore_after_close`: a node's
terminator is every byte between the end of what it renders as and the start
of what the next node owns. `Form::parse` then verifies the whole document
reproduces and refuses (`Reason::NotPreserved`) anything that does not, so no
file is ever accepted that would corrupt on save. The corpus fixture `denise-forms/tests/awkward/after-the-brace.dform` collects
every shape found so far, per the rule above.

`parse_form` asserted against `Reason::NotPreserved` while that hunt was on,
which is how both shapes surfaced. That assertion is gone now, because it
cannot be satisfied: kdl reads `before_ty_name`, `after_ty_name` and `after_ty`
off a node's type annotation and then writes none of them, so `(Z) h` comes
back as `(Z)h` and no format this crate can set will change it. Refusing is the
right answer there and refusing is not a finding. What is still asserted is the
half that matters — a form that *parsed* must round-trip.

### kdl parses some malformed files in exponential time

The second finding came from the `slow-unit-*` artifacts libFuzzer kept saving:
three-kilobyte inputs that took `kdl` **a second and a half to fail on**. The
cost is entirely inside `kdl` 6.7.1 (the newest release) — `Form::parse` adds
nothing measurable — and it is exponential, measured at **×1.78 per 229 bytes**
on two artifacts independently. Extrapolated, five kilobytes is twenty-four
seconds and eight kilobytes is hours. `MAX_SOURCE` is four *megabytes*, so it
was no protection at all.

Ablation narrowed the trigger to a slashdash and a children block together, and
the minimal shape is a commented-out node whose block has anything in it,
nested:

```kdl
/- a {
 b
```

Twenty of those is about a hundred bytes and twenty seconds; the sixty-four
`MAX_DEPTH` allowed would not have finished this century. `MAX_COMMENTED_DEPTH`
now refuses more than one such block inside another, by byte scan, before the
parser sees the file — which takes all four artifacts from 570–1400 ms to
0.00 ms while every form in the repository parses exactly as before. One level
is kept because commenting a widget and its children out is a real thing to do;
two changes nothing about what a file means.

### Strings the scan believed in and the parser did not

The limits are enforced by a byte scan, and a byte scan has to skip strings so
that a `{` written inside one is not counted as structure. Every skip is a
stretch of bytes not counted — so a string the scan believes in and `kdl` does
not is a hole straight through every limit at once. The fuzzer found two, twenty
minutes apart:

- **A quote with no partner.** The scan skipped to the end of the file.
  Thirty-four `#` and a quote at offset 501 hid twenty-four slashdashes and
  twenty-eight braces behind them. `kdl` does not stop reading there — it
  recovers and parses on.
- **A `"""` with no newline after it.** KDL spells a multi-line string `"""`
  and then a newline; `""" x` is a parse error to `kdl`, but the scan read it as
  a string opener and skipped past a hundred and twenty braces.
- **A closing `"""` that led no line.** `hi"""` on the end of a line does not
  close a multi-line string, and reading it as one skipped whatever came after.

The rule, now stated where it is enforced: **the scan should never recognise a
string that `kdl` would not.** Each of those is a condition on the scan now: a
quote must close, a `"""` must open a line, and its closer must lead one.

It is not a complete agreement, and this is where the chase was called off. KDL
also requires every line of a multi-line string to carry the closing `"""`'s
indentation, and a string that breaks *that* rule is an error to `kdl` and a
string to the scan — a fourth divergence, found five minutes after the third was
fixed. Matching `kdl` exactly means being `kdl`'s lexer, and each fix bought
about twenty minutes. What the conditions do guarantee is the direction of the
error: none of them can refuse a file that parses. A file that slips past them
reaches the parser and costs time, which is the same upstream problem as
everything else in this section rather than a new one.

The first of these was a hole in `MAX_DEPTH` from the day that guard was
written. Nothing had pushed on it before.

### Braces that do not balance

`MAX_COMMENTED_DEPTH` held for twenty minutes before the fuzzer found an input
it did not cover — nesting one commented block, well inside the limit, and a
second exponential family with it. Chasing shapes one at a time was losing.

The rule that actually covers them is older and duller: **a file whose braces
do not balance cannot parse**, whatever the parser does with it, so refusing it
up front is free. Every slow input found so far is wildly unbalanced — 26 `{`
against 1 `}` in the last one — and the check is two counters in a scan that
was already running. `Reason::Unbalanced` says which brace and where, which is
a better error than the recovery would have produced anyway.

**This is a bound on the shapes that were found, not a bound on the parser.**
`kdl` has other exponential corners: the commented-block limit needed
tightening from four levels to one to catch two of the four artifacts, and then
missed the next input outright. A balanced file that is malformed some other
way may well still be exponential, and nothing here would stop it. The complete fix belongs upstream. Until then, anything that parses a form it did not write should bound how long the
parse may take; `denise-forms` is `no_std + alloc` and has neither a thread nor
a clock to do that with, so it cannot be done in this crate.
