package dev.ytui.android;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import androidx.work.*;
import java.util.concurrent.*;
import org.json.*;

/** Resolves every playlist item off the UI thread; checkpointed between worker runs. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class PrepareWorker extends Worker {
  private volatile long activeRequest;

  public PrepareWorker(Context context, WorkerParameters parameters) {
    super(context, parameters);
  }

  static void enqueue(JSONObject request) {
    Data input = new Data.Builder().putString("request", request.toString()).build();
    OneTimeWorkRequest work =
        new OneTimeWorkRequest.Builder(PrepareWorker.class)
            .setInputData(input)
            .setBackoffCriteria(BackoffPolicy.EXPONENTIAL, 60, TimeUnit.SECONDS)
            .addTag("ytui-prepare")
            .build();
    status(MediaBridge.context, work.getId().toString(), "Preparing playlist", request.toString());
    WorkManager.getInstance(MediaBridge.context).enqueue(work);
  }

  static void status(Context context, String id, String text, String request) {
    context
        .getSharedPreferences("preparations", Context.MODE_PRIVATE)
        .edit()
        .putString(id, text)
        .putString(id + "-request", request)
        .apply();
  }

  @Override
  public Result doWork() {
    String id = getId().toString(), raw = getInputData().getString("request");
    if (raw == null) return Result.failure();
    java.io.File checkpoint =
        new java.io.File(getApplicationContext().getFilesDir(), "prepare-" + id + ".json");
    try {
      setForegroundAsync(foreground()).get(15, TimeUnit.SECONDS);
      JSONObject request = new JSONObject(raw);
      JSONArray items;
      if (checkpoint.isFile()) {
        try (java.io.InputStream in = new android.util.AtomicFile(checkpoint).openRead()) {
          java.io.ByteArrayOutputStream out = new java.io.ByteArrayOutputStream();
          byte[] b = new byte[8192];
          int n;
          while ((n = in.read(b)) != -1) {
            out.write(b, 0, n);
            if (out.size() > 16 * 1024 * 1024)
              throw new java.io.IOException("Playlist exceeds budget");
          }
          items = new JSONArray(out.toString("UTF-8"));
        }
      } else {
        JSONObject response =
            call(
                new JSONObject()
                    .put("op", "collection")
                    .put("request", request.get("request"))
                    .put("music", request.optBoolean("music")));
        items = response.getJSONArray("data");
        android.util.AtomicFile atomic = new android.util.AtomicFile(checkpoint);
        java.io.FileOutputStream out = null;
        try {
          out = atomic.startWrite();
          out.write(items.toString().getBytes(java.nio.charset.StandardCharsets.UTF_8));
          atomic.finishWrite(out);
        } catch (Exception error) {
          if (out != null) atomic.failWrite(out);
          throw error;
        }
      }
      SharedPreferences progress =
          getApplicationContext()
              .getSharedPreferences("preparations-progress", Context.MODE_PRIVATE);
      int start = PreparationPolicy.checkedCursor(progress.getInt(id, 0), items.length()),
          failed = progress.getInt(id + "-failed", 0);
      for (int i = start; i < items.length(); i++) {
        if (isStopped()) return Result.retry();
        JSONObject media = items.getJSONObject(i);
        status(
            getApplicationContext(),
            id,
            "Preparing " + (i + 1) + " / " + items.length() + " · " + failed + " failed",
            raw);
        JSONObject resolved =
            call(
                new JSONObject()
                    .put("op", "resolve")
                    .put("media", media)
                    .put("audio", request.optBoolean("audio"))
                    .put("quality", request.optString("quality", "1080")));
        if (isStopped()) return Result.retry();
        if (resolved.optBoolean("ok")
            && (media.optBoolean("live") || resolved.getJSONObject("data").optBoolean("live"))) {
          failed++;
          MediaBridge.event("error", "Skipped a live entry: recording is not supported");
        } else if (!resolved.optBoolean("ok")) {
          String error = resolved.optString("error").toLowerCase(java.util.Locale.ROOT);
          if (PreparationPolicy.rateLimited(error)) {
            status(
                getApplicationContext(),
                id,
                "Service cooldown; playlist preparation will retry",
                raw);
            return Result.retry();
          }
          failed++;
          MediaBridge.event(
              "error",
              "Playlist entry failed: "
                  + media.optString("title")
                  + " · "
                  + resolved.optString("error"));
        } else {
          JSONObject ready =
              new JSONObject()
                  .put("media", media)
                  .put("stream", resolved.getJSONObject("data"))
                  .put("audio", request.optBoolean("audio"));
          CompletableFuture<Void> accepted = new CompletableFuture<>();
          MediaBridge.main.post(
              () -> {
                try {
                  OfflineStore.add(ready);
                  accepted.complete(null);
                } catch (Exception e) {
                  accepted.completeExceptionally(e);
                }
              });
          accepted.get(15, TimeUnit.SECONDS);
          String downloadId = OfflineStore.id(ready);
          long deadline = android.os.SystemClock.elapsedRealtime() + 15000;
          while (!OfflineStore.persisted(downloadId, ready)) {
            if (isStopped() || android.os.SystemClock.elapsedRealtime() > deadline)
              throw new java.io.IOException(
                  "Download index did not acknowledge the playlist entry");
            Thread.sleep(100);
          }
        }
        if (!progress.edit().putInt(id, i + 1).putInt(id + "-failed", failed).commit())
          throw new java.io.IOException("Cannot persist playlist progress");
      }
      status(
          getApplicationContext(),
          id,
          failed == 0
              ? "Playlist queued: " + items.length() + " entries"
              : "Playlist queued with " + failed + " failures; review before retrying",
          raw);
      if (checkpoint.exists()) checkpoint.delete();
      return failed == 0 ? Result.success() : Result.failure();
    } catch (IllegalArgumentException error) {
      status(
          getApplicationContext(),
          id,
          "Invalid playlist checkpoint; remove and add the playlist again",
          raw);
      return Result.failure();
    } catch (Exception error) {
      status(
          getApplicationContext(),
          id,
          "Playlist preparation interrupted; retry from downloads",
          raw);
      return Result.retry();
    }
  }

  private JSONObject call(JSONObject request) throws Exception {
    activeRequest = System.nanoTime() & Long.MAX_VALUE;
    request.put("request_id", activeRequest);
    return new JSONObject(NativeBridge.request(request.toString()));
  }

  @Override
  public void onStopped() {
    super.onStopped();
    if (activeRequest > 0) NativeBridge.cancel(activeRequest);
  }

  @Override
  public com.google.common.util.concurrent.ListenableFuture<ForegroundInfo>
      getForegroundInfoAsync() {
    return com.google.common.util.concurrent.Futures.immediateFuture(foreground());
  }

  private ForegroundInfo foreground() {
    NotificationManager manager =
        getApplicationContext().getSystemService(NotificationManager.class);
    manager.createNotificationChannel(
        new NotificationChannel(
            "ytui-prepare", "Playlist preparation", NotificationManager.IMPORTANCE_LOW));
    Notification n =
        new Notification.Builder(getApplicationContext(), "ytui-prepare")
            .setSmallIcon(dev.ytui.android.R.drawable.ic_ytui)
            .setContentTitle("ytui: preparing playlist")
            .setContentText("Preparing every entry for offline download")
            .setOngoing(true)
            .build();
    return android.os.Build.VERSION.SDK_INT >= 29
        ? new ForegroundInfo(3, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        : new ForegroundInfo(3, n);
  }
}
