# Offline proxy TLS certificates

These are newly generated test credentials, unrelated to any service or person.
The deliberately public `key.der` is the dummy TLS server's unencrypted PKCS#8
RSA private key. It must never be used for a deployed service. Neither this CA
nor its private key is installed into the operating system. The CA private key
was discarded after generation.

Files:

- `ca.der`: self-signed RSA-2048/SHA-256 CA, CN `yt-dlp-rs offline proxy test CA`,
  critical CA constraint with path length zero and certificate/CRL signing usage.
- `server.der`: CA-signed RSA-2048/SHA-256 server certificate, CN
  `media.example.test`, SANs `DNS:localhost,DNS:media.example.test`, serverAuth EKU,
  critical non-CA and digital-signature/key-encipherment constraints.
- `key.der`: the corresponding dummy server private key, PKCS#8 DER.

Both certificates have fixed validity from **2020-01-01 00:00:00 UTC** through
**2050-01-01 00:00:00 UTC**, independent of checkout or test-run time. Generated
with OpenSSL 4.0.3 solely as development fixture preparation. Required tests
consume the saved DER directly through Rustls; they never execute OpenSSL or
make external network requests.

Generation used `openssl genpkey -algorithm RSA -pkeyopt rsa_keygen_bits:2048`
for each key and `openssl req -new` for the CA/server requests. A temporary
`openssl ca` database signed both requests with `-startdate 20200101000000Z
-enddate 20500101000000Z`; the CA request used `-selfsign`. The CA extensions were
`basicConstraints=critical,CA:TRUE,pathlen:0`,
`keyUsage=critical,keyCertSign,cRLSign`, and `subjectKeyIdentifier=hash`. Server
extensions were `basicConstraints=critical,CA:FALSE`,
`keyUsage=critical,digitalSignature,keyEncipherment`,
`extendedKeyUsage=serverAuth`, and the two DNS SANs above. A short-lived bootstrap
CA certificate was replaced with the fixed-validity CA before signing the server.
`openssl verify -CAfile ca.pem server.pem` passed. Certificates were converted
with `openssl x509 -outform DER`; the server key with
`openssl pkcs8 -topk8 -nocrypt -outform DER`.

SHA-256:

```text
a0168f6fc798557e8700a4a61eed469d36f167647e61044fe4e31540bd7c0949  ca.der
85b1a924d3b0de882c697ce4f4d17bdabff3e97f425b65aa72232f840cfb5900  server.der
aaff26ecf7bbb2c6dae308a16bcb2741d39e1f8a186778b8b34253fea73e948f  key.der
```

Two private test clients disable built-in roots and trust only this saved CA.
A third test uses the normal public library constructor and asserts certificate
verification rejects the same origin certificate. No production TLS verification
setting or root set is changed.
