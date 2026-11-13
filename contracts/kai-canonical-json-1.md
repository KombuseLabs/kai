# kai-canonical-json/1: one text for one JSON value, and its id

Not a file of its own: the rule by which kai turns a JSON value into one
text and names a record by that text's SHA-256. The ids of
[kai-environment/1](kai-environment-1.md), [kai-variant/1](kai-variant-1.md),
a study's content in [kai-lock/2](kai-lock-2.md) and a task's inventory
are made this way. `helpers::canonical` implements it. Example:
[kai-canonical-json-1.example.json](kai-canonical-json-1.example.json),
a value, its text and its id.

## The text

- **Objects:** members sorted by key, keys compared as UTF-8 bytes (the
  order of their code points). A member whose value is `null` is left
  out, so an unknown value and an absent one give the same text.
- **Arrays:** in their order. A `null` in an array stays.
- **No white space** outside strings.
- **Strings:** `"` and `\` escaped with `\`; the control characters
  U+0000 to U+001F as `\b`, `\t`, `\n`, `\f`, `\r` or `\u00XX` (lower-case
  hex); everything else as its UTF-8 bytes, without escapes.
- **Integers** (a number written without `.` and without an exponent, in
  the range of a signed or an unsigned 64-bit integer): in decimal, with
  `-` for a negative, without leading zeros.
- **Floats** (any other number): the fewest significant digits that read
  back to the same IEEE 754 double. With the value `m × 10^k`,
  `1 ≤ m < 10`:
  - for `-5 ≤ k ≤ 15`, positionally, always with a `.` and at least one
    digit after it: `0.2`, `100.0`, `0.00001`, `1500000000000000.0`;
  - otherwise as the digits of `m`, with a `.` only when there are several,
    then `e`, the sign (`+` or `-`) and `k` without leading zeros: `1e+16`,
    `1.5e+16`, `9.5e-6`, `5e-324`.

  `-0.0` is written `0.0`. A float is never the integer of the same value:
  `1.0` and `1` are different texts and different ids. NaN and the
  infinities have no JSON form; a file that holds one is refused.
- **Literals:** `true`, `false`, `null`.

## The id

`sha256:` followed by the SHA-256 of the text's UTF-8 bytes, in
lower-case hex. A record's id is not inside the record it names, and
neither are local paths or friendly names, which are aliases, not
identities.

Floats are a deliberate exception to "finite integers" in mvp-environments'
identity rules: a study's thresholds (`min_saving = 0.20`) are floats,
and the lock names the study by its id. The records that name runtimes and
variants hold no float.

A change of any rule that gives a value another text is
`kai-canonical-json/2`; an id made under `/1` stays what it was.
