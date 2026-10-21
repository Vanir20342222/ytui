# Automatic YouTube feed recovery — M4/M5, alpha.6

User report: YouTube sometimes loads only after manually refreshing. Scope includes
YouTube and Music Home, categories/search and continuation requests.

## Findings

- One network error left `TabView.error` set indefinitely, disabling prefetch.
- A duplicate-only page discarded even a valid advancing continuation token.
- Bootstrap client/visitor contexts were cached indefinitely.
- Home data already embedded in the bootstrap HTML was discarded.
- Refresh pushed the same feed onto history and reset the selection.

## Implemented behavior

1. Preserve the precise failed request, append mode and generation. The active tab
   retries transient transport, incomplete JSON and HTTP 408/425/429/5xx failures
   after 2/5/15/30/60 seconds, remaining at 60 seconds. Respect numeric/HTTP-date
   Retry-After. Wait in hidden tabs; cancel/replace on navigation or account change.
   Offline/demo modes never start recovery. Each service fetch has a 35-second
   deadline; ordinary HTTP requests retain their existing transport deadlines.
2. Keep loaded cards usable while loading/retrying. Display automatic recovery.
   Preserve the selected media ID when refreshing the same feed; keep Back history
   for actual navigation. Pending Refresh/Next keys cannot restart the request or
   bypass its cooldown. Consent, authentication and 401/403 need explicit action.
3. Keep WEB/Music contexts separate, expire them after 30 minutes and renew once
   for HTTP/API 400/412 context rejection. Read JSON embedded as `ytInitialData`
   without evaluating scripts. Use recognized Home content once within 60 seconds
   of bootstrap. Explicit service notices stay visible; bare Home shells renew
   once. Persistently bare signed-in Home schedules slower recovery; empty
   anonymous Home offers search/sign-in rather than polling indefinitely.
4. Prefetch against selection and the visible viewport. Deduplicate items while
   following new continuation tokens across overlap/empty pages; stop repeated or
   cyclic tokens and three consecutive pages without new items. Preserve selection.

No account mutations, browser imports, proxy/VPN changes or remote feed persistence
are introduced. This cannot bypass YouTube service/bot/region blocks.

## Verification

Deterministic loopback tests cover embedded Home in WEB/WEB_REMIX, single-use/freshness,
bare-shell recovery, explicit empty notices, stale/expired contexts, permanent versus
transient failures, incomplete JSON, API-level errors, numeric/date Retry-After and
transport failure. Controller tests cover automatic tick retry of the original
continuation, cooldown, hidden/offline behavior, stale generations, content/selection,
history, overlap/cycle bounds, viewport prefetch and duplicate Next actions.
Automatic recovery renders with content in all 13 themes at 80×24 and 180×50.

Final commands/results and live-network boundaries: [VALIDATION.md](VALIDATION.md).
