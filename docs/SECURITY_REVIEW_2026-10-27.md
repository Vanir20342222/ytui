# Secret-scanning review — 2026-10-27

The GitHub email identifies commit `36fd7b7`, the first Android implementation,
and Google API key matches in the vendored YouTube player fixture at lines 5817
and 6503. Those fixtures should have been sanitized before being vendored.

## Findings and correction

Both the challenge fixture and `sites/youtube-js/player.js` contained the same
two Google client keys in Google's public player code: certificate provisioning
and an X-Goog-Api-Key header. Before modification, the complete player fixture
was verified byte-for-byte against public upstream `aagedal/yt-dlp-rs` revision
`f67d96bdf98c5b821d0c9bf6175d328a0fa8ceab`. Its original SHA256 was
`ed9a4ea0646332a6b6c973ebe5a915c62c43db89bb2bb1a31eba07d906ef8ff7`.
These are upstream public client constants, not a developer's Google account,
GitHub credential or browser session. No key value is repeated in this report.

All four occurrences are replaced with inert, equal-length test placeholders.
The challenge algorithms and exact expected solution maps are retained. The
native and Android saved-player tests must continue to validate those maps.

Checksum-verified Gitleaks 8.30.1 scanned the current tracked source and all
locally fetched branches/history. The unfiltered review found **25 matches in
13 files**, all introduced by that vendored upstream commit. Every flagged file
was independently verified byte-for-byte against the pinned public upstream:

- Four Google-key occurrences in the two player copies, now sanitized.
- Three private-key matches in public, self-signed TLS test fixtures, used by the
  upstream local client-certificate integration test. They are not deployment
  certificates or ytui signing keys.
- Eighteen generic-detector matches in upstream public-client code and test data:
  HLS local key filenames, Blogger example URLs, ordinary JavaScript identifiers
  and template documentation. These are not personal ytui credentials.

No personal GitHub token, browser cookie or deployment credential was found by
this scan. This is a scanner result, not proof that every possible credential
format can be detected.

## Prevention and historical alerts

The Secret scan workflow verifies a pinned scanner archive checksum and scans all
fetched Git history on pushes and PRs with values fully redacted. The reviewed
public upstream findings are recorded as **exact historical fingerprints** in
`.gitleaksignore`: commit, file, detector and line. There are no directory-wide or
future-commit exemptions. A newly introduced secret, including a new occurrence
in the same fixture, is scanned normally. The original GitHub alert is not hidden
or automatically closed by this baseline.

The integration returns **HTTP 403** for GitHub's secret-scanning alerts API, so
alert status could not be inspected or changed here. After reviewing provenance,
the repository owner can resolve these particular alerts as **Used in tests**
with a link to this report and the sanitizing commit. Do not label them revoked:
ytui does not own or have permission to rotate Google's public client keys.

The old fixture bytes remain in Git history and upstream's public repository.
History was not force-rewritten. If another alert identifies an actual private
credential rather than these fixtures, revoke that credential first and review
its access history before considering coordinated history cleanup.
