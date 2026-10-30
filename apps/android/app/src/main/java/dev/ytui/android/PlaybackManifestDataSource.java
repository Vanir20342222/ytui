package dev.ytui.android;

import android.net.Uri;
import androidx.media3.datasource.*;
import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.util.*;

/** Request-local manifests without I/O; resource and subtitle reads retain their normal source. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class PlaybackManifestDataSource implements DataSource {
  static final String VIDEO = "ytui-memory://selected/video.mpd";
  static final String AUDIO = "ytui-memory://selected/audio.mpd";
  private final DataSource.Factory upstream;
  private final Map<String, byte[]> manifests;
  private final List<TransferListener> listeners = new ArrayList<>();
  private DataSource current;

  private PlaybackManifestDataSource(DataSource.Factory upstream, Map<String, byte[]> manifests) {
    this.upstream = upstream;
    this.manifests = manifests;
  }

  static DataSource.Factory factory(DataSource.Factory upstream, String video, String audio) {
    Map<String, byte[]> manifests = new HashMap<>();
    for (String[] entry : new String[][] {{VIDEO, video}, {AUDIO, audio}}) {
      if (!entry[1].isEmpty() && entry[1].length() <= 65536)
        manifests.put(entry[0], entry[1].getBytes(StandardCharsets.UTF_8));
    }
    return () -> new PlaybackManifestDataSource(upstream, manifests);
  }

  @Override
  public long open(DataSpec spec) throws IOException {
    byte[] manifest = manifests.get(spec.uri.toString());
    current = manifest == null ? upstream.createDataSource() : new ByteArrayDataSource(manifest);
    for (TransferListener listener : listeners) current.addTransferListener(listener);
    return current.open(spec);
  }

  @Override
  public int read(byte[] bytes, int offset, int length) throws IOException {
    return Objects.requireNonNull(current).read(bytes, offset, length);
  }

  @Override
  public Uri getUri() {
    return current == null ? null : current.getUri();
  }

  @Override
  public Map<String, List<String>> getResponseHeaders() {
    return current == null ? Collections.emptyMap() : current.getResponseHeaders();
  }

  @Override
  public void addTransferListener(TransferListener listener) {
    listeners.add(listener);
    if (current != null) current.addTransferListener(listener);
  }

  @Override
  public void close() throws IOException {
    try {
      if (current != null) current.close();
    } finally {
      current = null;
    }
  }
}
