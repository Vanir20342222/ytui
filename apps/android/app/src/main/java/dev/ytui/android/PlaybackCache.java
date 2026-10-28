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
    if (current != null) current.cancel(true);
    for (CacheWriter writer : writers.values()) writer.cancel();
    preloader.purge();
    upcomingUrl = "";
    ready = false;
  }

  static boolean allowed(JSONObject media) {
    if (!MobileSettings.enabled("preload_next")
        || media.optBoolean("live")
        || !media.isNull("offline_id")) return false;
    Uri uri = Uri.parse(media.optString("url"));
    String host = uri.getHost();
    if (MediaBridge.offlineUi
        && !Arrays.asList("127.0.0.1", "localhost", "::1").contains(host == null ? "" : host))
      return false;
    ConnectivityManager cm =
        (ConnectivityManager) MediaBridge.context.getSystemService(Context.CONNECTIVITY_SERVICE);
    return MobileSettings.enabled("preload_mobile") || !cm.isActiveNetworkMetered();
  }

  static synchronized void prefetch(JSONObject media, boolean audio, String quality) {
    if (!allowed(media)) {
      cancel();
      return;
    }
    String url = media.optString("url");
    if (url.equals(upcomingUrl) && current != null && (!current.isDone() || ready)) return;
    cancel();
    long token = intent.get();
    long account = NativeBridge.accountIntent();
    long network = NetworkSettings.revision.get();
    upcomingUrl = url;
    try {
      current =
          preloader.submit(
              () -> {
                try {
                  JSONObject stream =
                      PlaybackService.rpc(
                          new JSONObject()
                              .put("op", "resolve")
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
                }
              });
    } catch (RejectedExecutionException busy) {
      upcomingUrl = "";
    }
  }

  private static void warm(JSONObject stream, String url, int bytes, long token) throws Exception {
    if (token != intent.get()) return;
    CacheWriter writer =
        new CacheWriter(
            source(stream).createDataSource(),
            new DataSpec.Builder().setUri(url).setLength(bytes).build(),
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
