package dev.ytui.android;

import android.content.Context;
import android.net.*;
import androidx.media3.database.StandaloneDatabaseProvider;
import androidx.media3.datasource.*;
import androidx.media3.datasource.cache.*;
import java.util.*;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicLong;
import org.json.*;

/**
 * Bounded streaming LRU, separate from durable offline media. Prefetch never commits history/queue.
 */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class PlaybackCache {
  private static SimpleCache cache;
  private static final ThreadPoolExecutor preloader =
      new ThreadPoolExecutor(2, 2, 0L, TimeUnit.SECONDS, new ArrayBlockingQueue<>(1));
  private static final AtomicLong intent = new AtomicLong();
  private static Future<?> current;
  private static final AtomicLong requestIds = new AtomicLong(1_000_000_000L);
  private static volatile long activeRequest;
  private static String upcomingScope = "";
  private static final ConcurrentHashMap<Long, CacheWriter> writers = new ConcurrentHashMap<>();
  static volatile String upcomingUrl = "";
  static volatile boolean ready;

  static synchronized SimpleCache cache() {
    if (cache == null)
      cache =
          new SimpleCache(
              new java.io.File(MediaBridge.context.getCacheDir(), "streaming"),
              new LeastRecentlyUsedCacheEvictor(
                  MobileSettings.integer("stream_cache_mb") * 1024L * 1024),
              new StandaloneDatabaseProvider(MediaBridge.context));
    return cache;
  }

  static CacheDataSource.Factory source(JSONObject stream) {
    DefaultHttpDataSource.Factory http =
        new DefaultHttpDataSource.Factory()
            .setConnectTimeoutMs(8000)
            .setReadTimeoutMs(12000)
            .setAllowCrossProtocolRedirects(false)
            .setDefaultRequestProperties(PlaybackService.headers(stream.optJSONObject("headers")));
    return new CacheDataSource.Factory()
        .setCache(cache())
        .setFlags(CacheDataSource.FLAG_IGNORE_CACHE_ON_ERROR)
        .setUpstreamDataSourceFactory(new DefaultDataSource.Factory(MediaBridge.context, http));
  }

  static synchronized void cancel() {
    intent.incrementAndGet();
    if (activeRequest != 0) NativeBridge.cancel(activeRequest);
    if (current != null) current.cancel(true);
    for (CacheWriter writer : writers.values()) writer.cancel();
    preloader.purge();
    upcomingUrl = "";
    upcomingScope = "";
    ready = false;
  }

  static boolean allowed(JSONObject media) {
    if (!MobileSettings.enabled("preload_next")
        || media.optBoolean("live")
        || "station".equals(media.optString("kind"))
        || !media.isNull("offline_id")) return false;
    Uri uri = Uri.parse(media.optString("url"));
    String host = uri.getHost();
    if (MediaBridge.offlineUi
        && !Arrays.asList("127.0.0.1", "localhost", "::1").contains(host == null ? "" : host))
      return false;
    ConnectivityManager cm =
        (ConnectivityManager) MediaBridge.context.getSystemService(Context.CONNECTIVITY_SERVICE);
    return cm != null && (MobileSettings.enabled("preload_mobile") || !cm.isActiveNetworkMetered());
  }

  static synchronized void prefetch(JSONObject media, boolean audio, String quality) {
    if (!allowed(media)) {
      cancel();
      return;
    }
    String url = media.optString("url");
    String scope =
        audio
            + ":"
            + quality
            + ":"
            + NativeBridge.accountIntent()
            + ":"
            + NetworkSettings.revision.get();
    if (url.equals(upcomingUrl)
        && scope.equals(upcomingScope)
        && current != null
        && (!current.isDone() || ready)) return;
    cancel();
    long token = intent.get();
    long account = NativeBridge.accountIntent();
    long network = NetworkSettings.revision.get();
    upcomingUrl = url;
    upcomingScope = scope;
    long requestId = requestIds.incrementAndGet();
    activeRequest = requestId;
    try {
      current =
          preloader.submit(
              () -> {
                try {
                  // The current decoder's initial buffer takes priority over speculation.
                  if (!waitForBuffer(token)) return;
                  JSONObject stream =
                      PlaybackService.rpc(
                          new JSONObject()
                              .put("op", "resolve")
                              .put("request_id", requestId)
                              .put("speculative", true)
                              .put("media", media)
                              .put("audio", audio)
                              .put("quality", quality));
                  if (token != intent.get()
                      || account != NativeBridge.accountIntent()
                      || network != NetworkSettings.revision.get()) return;
                  warm(stream, stream.getString("url"), audio ? 384 * 1024 : 1024 * 1024, token);
                  String paired = stream.optString("audio_url");
                  if (!paired.isEmpty() && !paired.equals("null"))
                    warm(stream, paired, 192 * 1024, token);
                  if (token == intent.get()
                      && account == NativeBridge.accountIntent()
                      && network == NetworkSettings.revision.get()) {
                    ready = true;
                    MediaBridge.notifyUi();
                  }
                } catch (Exception ignored) {
                  /* speculative failure never interrupts current playback */
                } finally {
                  if (activeRequest == requestId) activeRequest = 0;
                }
              });
    } catch (RejectedExecutionException busy) {
      upcomingUrl = "";
    }
  }

  private static boolean waitForBuffer(long token) throws Exception {
    long until = android.os.SystemClock.elapsedRealtime() + 15000;
    while (token == intent.get() && !Thread.currentThread().isInterrupted()) {
      JSONObject status = new JSONObject(MediaBridge.playerSnapshot);
      if (!status.optBoolean("buffering")
          && (!status.optBoolean("playing") || status.optLong("buffered_ms") >= 2000)) return true;
      if (android.os.SystemClock.elapsedRealtime() >= until) return false;
      Thread.sleep(100);
    }
    return false;
  }

  private static void warm(JSONObject stream, String url, int bytes, long token) throws Exception {
    if (token != intent.get()) return;
    CacheWriter writer =
        new CacheWriter(
            source(stream).createDataSource(),
            new DataSpec.Builder()
                .setUri(url)
                .setKey(
                    stream.isNull(
                            url.equals(stream.optString("audio_url"))
                                ? "audio_cache_key"
                                : "cache_key")
                        ? null
                        : stream.optString(
                            url.equals(stream.optString("audio_url"))
                                ? "audio_cache_key"
                                : "cache_key"))
                .setLength(bytes)
                .build(),
            new byte[32 * 1024],
            (length, cached, added) -> {
              if (token != intent.get() || Thread.currentThread().isInterrupted())
                throw new CancellationException();
            });
    writers.put(token, writer);
    try {
      if (token != intent.get()) {
        writer.cancel();
        return;
      }
      writer.cache();
    } catch (java.io.EOFException shortMedia) {
      /* complete small media is already warm */
    } finally {
      writers.remove(token, writer);
    }
  }

  private PlaybackCache() {}
}
