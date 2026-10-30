package dev.ytui.android;

import android.net.Uri;
import androidx.media3.common.C;
import androidx.media3.datasource.*;
import java.io.IOException;
import java.util.*;
import java.util.function.Predicate;

/** Finite YouTube CDN ranges, with unchanged cache positions and a safe header fallback. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class IndexedRangeDataSource implements DataSource {
  private final HttpDataSource upstream;
  private final Predicate<Uri> host;
  private Uri original;
  private boolean rewritten;
  private long remaining;

  IndexedRangeDataSource(HttpDataSource upstream, Predicate<Uri> host) {
    this.upstream = upstream;
    this.host = host;
  }

  static DataSource.Factory factory(DefaultHttpDataSource.Factory http) {
    return () ->
        new IndexedRangeDataSource(http.createDataSource(), IndexedRangeDataSource::youtubeCdn);
  }

  static boolean youtubeCdn(Uri uri) {
    String name = uri.getHost();
    name = name == null ? "" : name.toLowerCase(Locale.ROOT);
    return ("https".equals(uri.getScheme()) || "http".equals(uri.getScheme()))
        && (name.equals("googlevideo.com") || name.endsWith(".googlevideo.com"))
        && "/videoplayback".equals(uri.getPath());
  }

  @Override
  public long open(DataSpec spec) throws IOException {
    original = spec.uri;
    rewritten = false;
    remaining = spec.length;
    boolean signedRange = false;
    for (String key : new String[] {"sparams", "lsparams"}) {
      String signed = spec.uri.getQueryParameter(key);
      if (signed != null && Arrays.asList(signed.split(",")).contains("range")) signedRange = true;
    }
    if (!host.test(spec.uri)
        || spec.httpMethod != DataSpec.HTTP_METHOD_GET
        || spec.httpBody != null
        || spec.httpRequestHeaders.keySet().stream().anyMatch(k -> k.equalsIgnoreCase("Range"))
        || spec.length <= 0
        || spec.uri.getQueryParameter("range") != null
        || signedRange
        || spec.position > Long.MAX_VALUE - spec.length) {
      return upstream.open(spec);
    }
    long expected = spec.length;
    String clen = spec.uri.getQueryParameter("clen");
    if (clen != null) {
      try {
        long size = Long.parseLong(clen);
        if (size <= spec.position) return upstream.open(spec);
        if (size > spec.position) expected = Math.min(expected, size - spec.position);
      } catch (NumberFormatException ignored) {
      }
    }
    Uri uri =
        spec.uri
            .buildUpon()
            .appendQueryParameter("range", spec.position + "-" + (spec.position + expected - 1))
            .build();
    // Query-ranged responses can use HTTP 200. Position zero prevents a second
    // client-side skip. An unbounded delegate spec also avoids a conflicting
    // Range header; this wrapper bounds reads to the original requested range.
    DataSpec query = spec.buildUpon().setUri(uri).setPosition(0).setLength(C.LENGTH_UNSET).build();
    long opened;
    try {
      opened = upstream.open(query);
    } catch (HttpDataSource.InvalidResponseCodeException e) {
      // A CDN may reject an added unsigned query argument. Preserve its
      // standard header path; do not retry rate limits or connectivity errors.
      if (e.responseCode != 400 && e.responseCode != 403 && e.responseCode != 416) throw e;
      upstream.close();
      return upstream.open(spec);
    }
    if (opened != expected) {
      // Missing/ignored query semantics must never return bytes at offset zero
      // for a non-zero request. Retry the standard HTTP Range form, unchanged.
      upstream.close();
      remaining = spec.length;
      return upstream.open(spec);
    }
    remaining = expected;
    rewritten = true;
    return expected;
  }

  @Override
  public int read(byte[] buffer, int offset, int length) throws IOException {
    if (length == 0) return 0;
    if (rewritten && remaining == 0) return C.RESULT_END_OF_INPUT;
    int read =
        upstream.read(buffer, offset, rewritten ? (int) Math.min(length, remaining) : length);
    if (rewritten && read == C.RESULT_END_OF_INPUT && remaining > 0)
      throw new java.io.EOFException("Indexed media range ended before its declared length");
    if (rewritten && read > 0) remaining -= read;
    return read;
  }

  @Override
  public Uri getUri() {
    return rewritten ? original : upstream.getUri();
  }

  @Override
  public Map<String, List<String>> getResponseHeaders() {
    return upstream.getResponseHeaders();
  }

  @Override
  public void addTransferListener(TransferListener listener) {
    upstream.addTransferListener(listener);
  }

  @Override
  public void close() throws IOException {
    try {
      upstream.close();
    } finally {
      original = null;
      rewritten = false;
    }
  }
}
