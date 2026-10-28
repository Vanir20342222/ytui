package dev.ytui.android;

import android.content.Context;
import androidx.media3.common.*;
import androidx.media3.database.StandaloneDatabaseProvider;
import androidx.media3.datasource.*;
import androidx.media3.datasource.cache.*;
import androidx.media3.exoplayer.offline.*;
import java.util.concurrent.Executors;
import org.json.*;

@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class OfflineStore {
  static final java.util.concurrent.ExecutorService IO = Executors.newFixedThreadPool(3);
  static SimpleCache cache;
  static DownloadManager manager;

  static synchronized DownloadManager manager() {
    if (manager == null) {
      Context ctx = MediaBridge.context;
      StandaloneDatabaseProvider db = new StandaloneDatabaseProvider(ctx);
      cache =
          new SimpleCache(
              new java.io.File(ctx.getFilesDir(), "offline"), new NoOpCacheEvictor(), db);
      DownloaderFactory factory =
          request -> {
            JSONObject data;
            try {
              data =
                  new JSONObject(new String(request.data, java.nio.charset.StandardCharsets.UTF_8));
            } catch (Exception ignored) {
              data = new JSONObject();
            }
            JSONObject stream = data.optJSONObject("stream");
            DefaultHttpDataSource.Factory http =
                new DefaultHttpDataSource.Factory()
                    .setDefaultRequestProperties(
                        PlaybackService.headers(
                            stream == null ? null : stream.optJSONObject("headers")))
                    .setConnectTimeoutMs(10000)
                    .setReadTimeoutMs(20000)
                    .setAllowCrossProtocolRedirects(false);
            return new DefaultDownloaderFactory(
                    new CacheDataSource.Factory()
                        .setCache(cache)
                        .setUpstreamDataSourceFactory(http),
                    IO)
                .createDownloader(request);
          };
      manager = new DownloadManager(ctx, new DefaultDownloadIndex(db), factory);
      manager.setMaxParallelDownloads(2);
      manager.setMinRetryCount(0);
      manager.resumeDownloads();
    }
    return manager;
  }

  static DataSource.Factory offlineFactory() {
    manager();
    return new CacheDataSource.Factory()
        .setCache(cache)
        .setUpstreamDataSourceFactory(null)
        .setCacheWriteDataSinkFactory(null);
  }

  static String id(JSONObject request) throws Exception {
    byte[] hash =
        java.security.MessageDigest.getInstance("SHA-256")
            .digest(
                request
                    .getJSONObject("media")
                    .getString("url")
                    .getBytes(java.nio.charset.StandardCharsets.UTF_8));
    StringBuilder hex = new StringBuilder();
    for (byte b : hash) hex.append(String.format(java.util.Locale.ROOT, "%02x", b));
    return hex.toString();
  }

  static void add(JSONObject request) throws Exception {
    if (request.getJSONObject("media").optBoolean("live")
        || request.getJSONObject("stream").optBoolean("live"))
      throw new IllegalArgumentException("Live recording is not available");
    manager();
    JSONObject stream = request.getJSONObject("stream");
    String id = id(request);
    // Persist the selected streams, not the entire format/caption catalog.
    // Snapshot polling parses this data; large catalogs multiplied by a playlist
    // otherwise cost memory/CPU on every refresh and contain unusable captions.
    JSONObject compact = new JSONObject(request.toString());
    compact.getJSONObject("stream").remove("catalog");
    byte[] data = compact.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8);
    DownloadRequest main =
        new DownloadRequest.Builder(id, android.net.Uri.parse(stream.getString("url")))
            .setData(data)
            .build();
    DownloadService.sendAddDownload(MediaBridge.context, OfflineService.class, main, true);
    String audio = stream.optString("audio_url", "");
    if (!audio.isEmpty() && !"null".equals(audio)) {
      DownloadService.sendAddDownload(
          MediaBridge.context,
          OfflineService.class,
          new DownloadRequest.Builder(id + "-audio", android.net.Uri.parse(audio))
              .setData(data)
              .build(),
          true);
    }
  }

  static boolean persisted(String id, JSONObject request) throws Exception {
    if (manager().getDownloadIndex().getDownload(id) == null) return false;
    String audio = request.getJSONObject("stream").optString("audio_url", "");
    return audio.isEmpty()
        || "null".equals(audio)
        || manager.getDownloadIndex().getDownload(id + "-audio") != null;
  }

  static JSONObject offlineRequest(String id) throws Exception {
    Download item = manager().getDownloadIndex().getDownload(id);
    if (item == null || item.state != Download.STATE_COMPLETED)
      throw new Exception("Download is not complete");
    JSONObject request =
        new JSONObject(new String(item.request.data, java.nio.charset.StandardCharsets.UTF_8));
    String audio = request.getJSONObject("stream").optString("audio_url", "");
    if (!audio.isEmpty() && !"null".equals(audio)) {
      Download paired = manager.getDownloadIndex().getDownload(id + "-audio");
      if (paired == null || paired.state != Download.STATE_COMPLETED)
        throw new Exception("Audio track is not complete");
    }
    request.put("offline", true);
    return request;
  }

  static String label(int state) {
    switch (state) {
      case Download.STATE_QUEUED:
        return "Queued";
      case Download.STATE_STOPPED:
        return "Paused";
      case Download.STATE_DOWNLOADING:
        return "Downloading";
      case Download.STATE_COMPLETED:
        return "Complete";
      case Download.STATE_FAILED:
        return "Failed";
      case Download.STATE_REMOVING:
        return "Removing";
      case Download.STATE_RESTARTING:
        return "Restarting";
      default:
        return "Pending";
    }
  }

  private static volatile String cachedSnapshot = "[]";
  private static volatile long snapshotVersion = 0, lastRefresh = 0;
  private static long deliveredVersion = -1;
  private static final java.util.concurrent.atomic.AtomicBoolean refreshing =
      new java.util.concurrent.atomic.AtomicBoolean();

  static void resetSnapshotClient() {
    deliveredVersion = -1;
  }

  // Database/prefs enumeration never runs in the Rust render thread.
  static JSONArray changedSnapshot() throws JSONException {
    long now = android.os.SystemClock.elapsedRealtime();
    if (now - lastRefresh > 1000 && refreshing.compareAndSet(false, true))
      IO.execute(
          () -> {
            try {
              String current = snapshot().toString();
              if (!current.equals(cachedSnapshot)) {
                cachedSnapshot = current;
                snapshotVersion++;
              }
              lastRefresh = android.os.SystemClock.elapsedRealtime();
            } finally {
              refreshing.set(false);
            }
          });
    long version = snapshotVersion;
    if (deliveredVersion == version) return null;
    JSONArray result = new JSONArray(cachedSnapshot);
    deliveredVersion = version;
    return result;
  }

  static JSONArray snapshot() {
    JSONArray rows = new JSONArray();
    try {
      if (manager != null) {
        try (DownloadCursor cursor = manager.getDownloadIndex().getDownloads()) {
          while (cursor.moveToNext() && rows.length() < 2000) {
            Download d = cursor.getDownload();
            if (d.request.id.endsWith("-audio")) continue;
            JSONObject request =
                new JSONObject(new String(d.request.data, java.nio.charset.StandardCharsets.UTF_8));
            String state = label(d.state);
            double progress = Math.max(0, d.getPercentDownloaded()) / 100.0;
            String audio = request.getJSONObject("stream").optString("audio_url", "");
            if (!audio.isEmpty() && !"null".equals(audio)) {
              Download paired = manager.getDownloadIndex().getDownload(d.request.id + "-audio");
              progress =
                  (progress
                          + (paired == null
                              ? 0
                              : Math.max(0, paired.getPercentDownloaded()) / 100.0))
                      / 2;
              if (d.state == Download.STATE_COMPLETED
                  && (paired == null || paired.state != Download.STATE_COMPLETED))
                state =
                    paired != null && paired.state == Download.STATE_FAILED
                        ? "Audio failed"
                        : "Waiting for audio";
            }
            rows.put(
                new JSONObject()
                    .put("id", d.request.id)
                    .put("title", request.getJSONObject("media").optString("title"))
                    .put("state", state)
                    .put("progress", progress));
          }
        }
      }
      for (java.util.Map.Entry<String, ?> entry :
          MediaBridge.context
              .getSharedPreferences("exports", Context.MODE_PRIVATE)
              .getAll()
              .entrySet()) {
        if (rows.length() >= 2016) break;
        rows.put(
            new JSONObject()
                .put("id", "export:" + entry.getKey())
                .put("title", "Media export")
                .put("state", String.valueOf(entry.getValue()))
                .put("progress", 0));
      }
      for (java.util.Map.Entry<String, ?> entry :
          MediaBridge.context
              .getSharedPreferences("preparations", Context.MODE_PRIVATE)
              .getAll()
              .entrySet()) {
        if (entry.getKey().endsWith("-request") || rows.length() > 2016) continue;
        rows.put(
            new JSONObject()
                .put("id", "job:" + entry.getKey())
                .put("title", "Playlist preparation")
                .put("state", String.valueOf(entry.getValue()))
                .put("progress", 0));
      }
    } catch (Exception ignored) {
    }
    return rows;
  }

  static void remove(String id) {
    if (id.startsWith("export:")) {
      String job = id.substring(7);
      MediaBridge.context
          .getSharedPreferences("export-dismissed", Context.MODE_PRIVATE)
          .edit()
          .putBoolean(job, true)
          .commit();
      androidx.work.WorkManager.getInstance(MediaBridge.context)
          .cancelWorkById(java.util.UUID.fromString(job));
      MediaBridge.context
          .getSharedPreferences("exports", Context.MODE_PRIVATE)
          .edit()
          .remove(job)
          .apply();
      return;
    }
    if (id.startsWith("job:")) {
      String job = id.substring(4);
      androidx.work.WorkManager.getInstance(MediaBridge.context)
          .cancelWorkById(java.util.UUID.fromString(job));
      MediaBridge.context
          .getSharedPreferences("preparations", Context.MODE_PRIVATE)
          .edit()
          .remove(job)
          .remove(job + "-request")
          .apply();
      return;
    }
    DownloadService.sendRemoveDownload(MediaBridge.context, OfflineService.class, id, false);
    DownloadService.sendRemoveDownload(
        MediaBridge.context, OfflineService.class, id + "-audio", false);
  }

  static void toggle(String id) throws Exception {
    if (id.startsWith("export:")) return;
    if (id.startsWith("job:")) {
      String job = id.substring(4);
      String raw =
          MediaBridge.context
              .getSharedPreferences("preparations", Context.MODE_PRIVATE)
              .getString(job + "-request", null);
      androidx.work.WorkManager.getInstance(MediaBridge.context)
          .cancelWorkById(java.util.UUID.fromString(job));
      if (raw != null) PrepareWorker.enqueue(new JSONObject(raw));
      return;
    }
    Download main = manager().getDownloadIndex().getDownload(id);
    Download paired = manager.getDownloadIndex().getDownload(id + "-audio");
    if (main != null
        && (main.state == Download.STATE_FAILED
            || (paired != null && paired.state == Download.STATE_FAILED))) {
      JSONObject request =
          new JSONObject(new String(main.request.data, java.nio.charset.StandardCharsets.UTF_8));
      IO.execute(
          () -> {
            try {
              JSONObject stream =
                  PlaybackService.rpc(
                      new JSONObject()
                          .put("op", "resolve")
                          .put("media", request.getJSONObject("media"))
                          .put("audio", request.optBoolean("audio"))
                          .put("quality", request.opt("quality")));
              request.put("stream", stream);
              MediaBridge.main.post(
                  () -> {
                    try {
                      add(request);
                    } catch (Exception e) {
                      MediaBridge.event("error", "Could not restart download");
                    }
                  });
            } catch (Exception error) {
              MediaBridge.event("error", "Download retry could not refresh the stream");
            }
          });
      return;
    }
    for (String key : new String[] {id, id + "-audio"}) {
      Download d = manager().getDownloadIndex().getDownload(key);
      if (d == null) continue;
      if (d.state == Download.STATE_FAILED)
        DownloadService.sendAddDownload(MediaBridge.context, OfflineService.class, d.request, true);
      else
        DownloadService.sendSetStopReason(
            MediaBridge.context, OfflineService.class, key, d.stopReason == 0 ? 1 : 0, true);
    }
  }

  private OfflineStore() {}
}
