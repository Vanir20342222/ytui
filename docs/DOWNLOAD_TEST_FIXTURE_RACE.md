# Download-recovery fixture launch race (M8)

During Android UI qualification, Rust run
[41112482759, attempt 1](https://github.com/Vanir20342222/ytui/actions/runs/41112482759/attempts/1)
failed `temporary_playlist_failure_retries_whole_source_and_keeps_completed_outputs`
with `Error` instead of `Done`. The application/download code was unchanged.
The same head passed a rerun; that rerun does not explain or repair the failure.

A temporary diagnostic probe reproduced the same failure, plus missing-output
and cancellation-readiness failures, when launching newly written fake shell
executables in parallel tests. The concrete launch error was Linux **ETXTBSY**
(errno 26, `Text file busy`). The ordinary test error had retained only its
`Cannot start ...` context, so the retry assertion hid the launch cause.

500 isolated retries passed. Full-suite repetitions reproduced failures, including
a cancellation fixture exiting before its ready marker. Atomic publication of
the scripts alone still reproduced the launch error. A fixture-lifetime mutex
prevents one fixture's file creation and subprocess activity overlapping another
fixture in this integration-test process. Each fixture's worker, pipes, event
stream and cancellation remain concurrent; all assertions, retry limits,
source/output checks and five-second worker deadline are unchanged. Failure
messages now include the stored error and retry events.

The actual corrected `download_recovery` binary passes **1,000 full-suite runs**
with **eight test threads**: 11 scenarios per run, 11,000 repeated scenario
executions, zero failures (33.75 seconds in this workspace). These repetitions
are a diagnostic stress check, not 11,000 distinct tests or live YouTube evidence.
Formatting and focused strict Clippy are checked separately; CI provides the
workspace/media regressions.

No production process launcher, HTTP/retry policy, downloader, Android UI or data
format changes. Temporary diagnostic instrumentation/probe source was removed
from the UI workspace. This test-only fix is submitted separately from UI PR #29.
