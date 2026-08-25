# The awkward corpus

Form files written the way a person writes them, not the way a designer would.
Every one of them must survive **open → save with no edits, byte for byte**, and
that is what [#88](https://github.com/bisand/denise/issues/88) is about: files are
meant to be edited by hand *and* by the designer, alternately, in the same
repository, and that only works if a save is a no-op on everything nobody touched.

Delphi's `.dfm` got this mostly right. XAML designers mostly did not, and people
stopped trusting them.

These are fixtures, not examples. If you want to see what a form *should* look
like, read [`forms/reference.dform`](../../../forms/reference.dform). Everything
here is a deliberate mess with a comment at the top saying which mess it is.

Two tests load all of them:

- `denise-forms/tests/awkward.rs` — through `Form::parse` and `Form::text`, the
  format's own round trip, plus one targeted edit each.
- `tools/designer/src/app.rs` — through `Document::open` and `Document::save`,
  which is the path a person's file actually takes.

Adding a file here is how a new way of writing a form by hand gets defended. No
test needs changing: both walk the directory.
