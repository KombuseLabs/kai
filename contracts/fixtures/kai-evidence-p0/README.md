# Kai contract conformance fixtures (P0)

All data in this corpus is invented. The complete pair starts from the existing byte-exact synthetic Reporter fixture; SDK event envelopes and external ledgers were added as synthetic bytes. No provider was called and no real prompt, session, account, host path, or usage is represented.

`report-evidence.json` demonstrates the proposed `kai-report-evidence/2` wire shape. `cases.json` is the machine-readable conformance vector list for the upcoming producer/reader and adapter work. Expected values are contract decisions, not evidence that those future adapters are implemented.

The same corpus bytes are checked into the producer's `contracts/fixtures/kai-evidence-p0/` and the consumer's `tests/fixtures/kai-evidence-p0/`; `SHA256SUMS` binds the copied bytes. Update both copies together and verify with `sha256sum -c SHA256SUMS`.

Privacy boundary: source transcripts, Sieve ledgers, proxy ledgers, session ids and source line/byte references remain local. Only explicitly allowlisted aggregate evidence may enter the later portable report.
