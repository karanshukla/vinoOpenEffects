# Comment style

A comment is the last resort. Work down this ladder, and write prose only
when all four rungs fail.

1. **Extract it.** Logic that a comment explains belongs in a function with
   a domain name. Arithmetic spelled out in a comment belongs in named
   constants. A coercion that is explained twice belongs in one named helper.

   ```rust
   // Before: the Rec. 601 weights, written out at each use
   let lum = (77 * img[p] as u32 + 150 * img[p + 1] as u32 + 29 * img[p + 2] as u32) as f32 / 65280.0;
   if crop.zf >= 0.999 {

   // After
   let lum = luma_x256(&img[p..]) as f32 / LUMA_X256_MAX;
   if crop.zf >= FULL_FRAME {
   ```

2. **Name it.** A comment that labels a block (`// Phase 2: fit the
   coefficients`) means the block wants to be a function. Sentinels and
   tuples get names too: `missed + 1 < MAX_MISSES`, not `< 2 // dropped after
   two`; a `Subject { cx, cy, span, kind }`, not `(f32, f32, f32, SubjectKind)`
   with a doc comment listing the fields.

3. **Pin it with a test pair.** A comment that states a business rule is a
   rule nothing enforces. Write one test inside the boundary and one
   outside, named after the rule (`drift_inside_the_comfort_zone_never_reframes`
   / `sustained_drift_reframes_after_the_confirm_window`), instead of
   writing "natural movement never reframes". The pair is the point: it
   fails the day someone moves the limit. In a test, a comment that labels
   the next few asserts is the same smell. Split those asserts into their
   own test, named after what the comment said.

4. **Pin cross-boundary rules with an end-to-end test.** The D-Bus
   interface, the CLI, `state.toml` round-trips and what `install.sh`
   writes belong in `tests/integration/`, the `dbus_server.rs` peer-to-peer
   tests, or the CI install job, not in a paragraph.

5. **Only then, prose.** Hidden constraints, upstream bugs, protocol
   requirements, incident history that still constrains the code. State the
   constraint, not its biography: "libspa-rs has no typed enum for these
   keys" is enough without the story of how we found out, and an issue
   number is not a reason. Measured claims carry a date.

## Linking the test from the code

When rung 3 or 4 replaced a comment, the code it governs says which test
pins the rule, in its doc comment, always with the same phrase:

```rust
/// Commit `raw` as the target only once it has stayed outside the comfort
/// zone for the confirmation window (pinned by
/// `tests::drift_inside_the_comfort_zone_never_reframes`,
/// `tests::sustained_drift_reframes_after_the_confirm_window`).
```

`tests::name` (or `widget_tests::name`, `golden_tests::name`) is a test
module of the file's own module; `a::b::tests::name` is one under the
crate root.

rustdoc can't link to `#[test]` functions (they are compiled out even
under `--cfg test`), so these are code spans, not intra-doc links.
`shared/tests/doc_links.rs` (CI job "Doc Links") checks them in every
crate instead: each code span after "pinned by", up to the closing `)`,
must be a `#[test]` fn. The phrase is one constant there.

## Exempt

- `///` and `//!` docs on items, which say what an item is and how to call
  it. They follow the same "constraint, not biography" rule, and don't
  restate the signature, but aren't replaced by code. clap's `///` on
  `cli/src/args.rs` is `--help` text.
- `// SAFETY:` comments on `unsafe`, `#[allow]` reasons, and coverage
  pragmas.
