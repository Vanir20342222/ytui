# Saved proxy configuration references

Official pinned yt-dlp: `897341241f2bfb2439fe0367f4c7c9ccd29a3828`.
Run `python3 tools/compare_proxies.py --upstream /path/to/pinned/yt-dlp`.

`cases.json` contains 50 source configurations and eight additional explicitly
source-only cases. Of the 50, native consumes 39 explicit endpoint configurations,
rejects five SOCKS4 credential configurations, and treats six omitted-option
cases only as an Environment mode marker. Those eleven configurations carry
`native_policy`; their actual source results remain unchanged reference data.
Each has `args`, a controlled `environment` proxy mapping, request URLs and
`expected` (`proxy_option`, `selected` endpoints and remaining `headers`).
`proxy_option` preserves the actual explicit CLI value or null when omitted.
Each selected endpoint is null (direct) or an object with scheme, host, port,
username, password and rdns. Native fixture tests compare supported explicit
endpoints without network or environment mutations. Environment configurations
do not assert route equality. Environment keys are the already-discovered urllib
getproxies mapping, not raw OS environment variables.

The worker calls actual parse_options, YoutubeDL.proxies, clean_proxies,
select_proxy and make_socks_proxy_opts. It replaces both standard-library and
upstream compat urllib request getproxies functions with the supplied map, and
stubs system proxy bypass false. Actual `proxy_bypass_environment` still handles
the supplied `no` value. This avoids ambient environment, OS proxy settings,
PAC/browser configuration and user config files. Sockets, downloads and child
processes are forbidden. Configuration decisions do not prove proxy transport,
TLS negotiation, CONNECT behavior, DNS routing or authentication wire behavior.

Endpoint projection is explicitly documented:

- Host is actual urllib parsed hostname (case-insensitive, no IPv6 brackets).
  Scheme comes after actual source cleanup; raw URL casing/trailing slash is not
  compared, and supported explicit URLs have no meaningful path/query/fragment.
- SOCKS host/port, decoded credentials and rdns come from actual
  make_socks_proxy_opts. Default port is 1080. Source clean_proxies first converts
  socks to socks4 and socks5 to socks5h. Thus ordinary CLI socks5 requests remote
  DNS even though the uncleaned helper's socks5 scheme would use local DNS.
- HTTP(S) port is explicit parsed port, or the standard library's
  HTTPConnection/HTTPSConnection.default_port (80/443). This is canonical endpoint
  comparison, not a claim about all upstream backend defaults. HTTP credentials
  use actual urllib username/password parsing and percent-unquote; null and empty
  remain distinct. Neither source requests nor authentication headers are sent.
- Names, decoded credentials and values are not removed or masked in the saved
  synthetic fixture. All credentials are intentionally fake reference values.

Source configurations verify absent proxy/no environment, explicit empty and __noproxy__
direct, repeated last option wins, missing http scheme, scheme-relative proxy
URLs, IPv6, scheme defaults, supported schemes/aliases and percent-decoded/user-
only/empty credentials. Explicit proxy overrides all environment including no.
An omitted explicit option uses the actual source discovered environment mapping: HTTP
falls back to HTTPS only if HTTPS is absent; all provides general fallback;
no bypasses matching hosts; HTTPS __noproxy__ remains direct. Native preserves
reqwest environment behavior and compares only the Environment mode marker for
these six omitted-option cases. Python HTTP-to-HTTPS fallback, no_proxy routing
and platform discovery are reference evidence, not native route parity claims.

Native rejects the five SOCKS4 userinfo configurations because its reqwest
transport cannot carry a SOCKS4 USERID. Their parsed source credentials remain
reference-only. SOCKS5 incomplete or empty credentials remain represented in the
parsed URL for endpoint comparison, but native clears userinfo when either
username or password is empty/missing before transport. Actual upstream
`socks.py:183` likewise offers username/password authentication only when both
are nonempty (it also offers no authentication).
Configuration projection therefore does not claim authentication bytes or
wire-level credential behavior.

Source-only cases make strict native validation boundaries explicit. Upstream
cleanup preserves non-root paths, query, fragments, malformed percent escapes,
whitespace, controls and unsupported schemes until later backend handling. The
reference records cleanup/selection results, not successful transport for them.
Native rejects these with generic proxy errors, along with its own bounds and
invalid host/port syntax. No credentials should appear in native errors.
The legacy Ytdl-Request-Proxy header is also source-only: source cleanup removes
it and replaces every configured route, even no_proxy; native rejects it rather
than admitting an implicit per-request proxy feature.

Source references:

- `yt_dlp/options.py:609-613`: --proxy, empty string direct and SOCKS syntax;
  `654-658` is the separate unsupported geo-verification-proxy feature.
- `yt_dlp/YoutubeDL.py:4205-4218`: explicit option/environment selection, direct
  sentinel and HTTP-to-HTTPS compatibility fallback.
- `yt_dlp/utils/networking.py:169-196`: clean_proxies, request override header,
  direct sentinel, missing-scheme handling and SOCKS aliases.
- `yt_dlp/utils/networking.py:246-258`: select_proxy, controlled no_proxy/system
  bypass then scheme/all selection.
- `yt_dlp/networking/_helper.py:53-90`: actual SOCKS type, default port, rdns and
  percent-decoded credential options.
- `yt_dlp/compat/urllib/request.py`: compat module re-exports getproxies and
  contains separate Windows discovery handling, not exercised here.

These mapped files contain many additional request, TLS, proxy backend,
geo-bypass and compatibility behaviors outside this bounded configuration slice.
