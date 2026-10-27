# Site extractor fixtures

Each directory holds one site's saved responses (`routes.json`, keyed by full
URL, with bodies inline or in sibling files), the CLI cases to run
(`cases.json`) and the pinned upstream's output for them (`expected.json`,
from `tools/capture_sites.py --upstream CHECKOUT --write`).

Both upstream and this port run through a local proxy that serves only these
routes (`tools/site_proxy.py`, `tests/helpers/site_proxy.rs`): plain requests
by absolute URI and HTTPS through `CONNECT`, terminated with
`tests/fixtures/proxies/tls`'s self-signed certificate, so the runs use
`--proxy PROXY --no-check-certificate --user-agent fixture`. API responses are
synthetic, shaped like the sites' documented responses and upstream's tests;
they do not prove live-site compatibility.
