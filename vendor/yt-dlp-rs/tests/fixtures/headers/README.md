# Saved HTTP request-header references

Official pinned source: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.
Run `python3 tools/compare_headers.py --upstream /path/to/pinned/yt-dlp`.
The worker calls actual `parse_options`, `HTTPHeaderDict` and
`YoutubeDL._calc_headers`, with sockets, downloads and subprocesses forbidden.
Explicit option argv avoids user configuration. No HTTP requests are made.

`cases.json` contains:

- `cases`: 21 `{name,args,expected_headers}` configurations for pure native
  configuration tests. Expected headers are a lowercase-key object.
- `request_cases`: seven actual `_calc_headers` merges with explicit fixed
  `defaults`, `global`, `extractor` and `expected_headers`. The `cookie-removed`
  case has `native_policy` and is source-only; exclude it from native parity
  assertions. Defaults deliberately use `Fixture-Default-UA` rather than claiming
  randomized upstream browser UA or native default UA equality.
- `invalid_cases`: seven source CLI grammar rejections, verified as actual
  `OptParseError`. Native validates header grammar/transport separately.
- `source_only_cases`: Cookie compatibility, Unicode field names and embedded
  CR/LF values accepted by the source option/header container. Each includes an
  explicit native-policy explanation; these are not supported native inputs.

The sole comparison projection lowercases field names because HTTP header names
are case-insensitive. Values remain exact actual upstream output, including
empty values and internal spaces. Header object insertion order is not a wire
ordering claim. Case-insensitive duplicates use the last custom value. Source
splits only at the first colon, preserves colons within values and trims values
when constructing `HTTPHeaderDict`; its option callback itself preserves values.
No payload/header-size, network transport, redirect forwarding or site-specific
extractor behavior is inferred from these pure fixtures.

The convenience flags have a source quirk: `--add-headers` lowercases field keys,
then final option processing uses `setdefault('User-Agent', ...)` and
`setdefault('Referer', ...)` with uppercase keys. Those later entries win when
converted to case-insensitive `HTTPHeaderDict`, regardless of CLI ordering.
Empty convenience values also win. Repeated convenience flags use their last
value. The singular `--add-header` works as upstream optparse's unambiguous
abbreviation of `--add-headers`; its explicit native alias does not claim general
option abbreviation compatibility.

Actual request merging applies fixed defaults, global options, then extractor
headers. Extractor Referer/User-Agent/custom values override global values,
including empty ones. These are request dictionary semantics only; native
extractors and transport have separate local request/header tests.

Cookie is excluded from the native feature because raw Cookie compatibility loading is not implemented.
Upstream `YoutubeDL` compatibility-loads a user Cookie into its jar and removes
it from generic headers; `_calc_headers` strips Cookie again to avoid leaking
unscoped cookies. The reference's source-only request case uses an empty actual
`YoutubeDLCookieJar` and does not enable compatibility cookie loading. It proves
removal without claiming scoped jar behavior, cookie file support or forwarding.

Source option regex allows Unicode word characters and rejects several otherwise
valid HTTP token punctuation characters. Native explicitly rejects non-ASCII
field names, unsafe/control values and Cookie, and may enforce stronger bounded
configuration rules. Upstream option parsing/container acceptance is not proof
that its network transport would accept an invalid CR/LF value. Native limits
and transport validation are independent tests; no unsafe input is sent.

Source references:

- `yt_dlp/options.py:268-297`: dictionary option callback, anchored key syntax,
  first colon, lowercase keys and repeated-value replacement.
- `yt_dlp/options.py:1186-1200`: user-agent, referer and add-headers definitions.
- `yt_dlp/__init__.py:477-480,914`: convenience option finalization and
  `http_headers` parameter mapping.
- `yt_dlp/utils/networking.py:24-105`: actual `HTTPHeaderDict` case-insensitive
  merge, final key casing and value trimming.
- `yt_dlp/YoutubeDL.py:746-748`: default/global merge and compatibility cookies;
  `2686-2720`: `_calc_headers`, extractor precedence and Cookie removal.
- `yt_dlp/YoutubeDL.py:2982-2989`: format/root header processing before downloads;
  `4490-4491`: subtitle headers inherit parent headers when absent.

These files contain broader cookie, proxy, authentication, network, site-header
and browser impersonation behavior outside this bounded configuration slice.
