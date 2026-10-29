package dev.ytui.android;

import java.net.*;
import java.util.*;
import org.json.JSONObject;

/** Apply the same explicit HTTP proxy to platform media as to Rust requests. */
final class NetworkSettings {
  private static final ProxySelector original = ProxySelector.getDefault();
  static volatile boolean explicitProxy;
  static final java.util.concurrent.atomic.AtomicLong revision =
      new java.util.concurrent.atomic.AtomicLong();
  private static String applied = "";

  static synchronized void apply(JSONObject config) throws Exception {
    String value = config.optString("proxy", "");
    if (value.equals("null")) value = "";
    if (value.equals(applied)) return;
    if (value.isEmpty() || value.equals("null")) {
      ProxySelector.setDefault(original);
      explicitProxy = false;
      applied = "";
      revision.incrementAndGet();
      PlaybackCache.cancel();
      return;
    }
    URI uri = new URI(value);
    if (!"http".equals(uri.getScheme()) || uri.getUserInfo() != null || uri.getHost() == null)
      throw new IllegalArgumentException("Unsupported Android media proxy");
    Proxy proxy =
        new Proxy(
            Proxy.Type.HTTP,
            InetSocketAddress.createUnresolved(
                uri.getHost(), uri.getPort() > 0 ? uri.getPort() : 80));
    ProxySelector.setDefault(
        new ProxySelector() {
          @Override
          public List<Proxy> select(URI target) {
            return Collections.singletonList(proxy);
          }

          @Override
          public void connectFailed(URI target, SocketAddress address, java.io.IOException error) {}
        });
    explicitProxy = true;
    applied = value;
    revision.incrementAndGet();
    PlaybackCache.cancel();
  }

  private NetworkSettings() {}
}
