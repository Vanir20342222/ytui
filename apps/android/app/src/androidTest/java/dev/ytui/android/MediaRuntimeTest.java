package dev.ytui.android;

import static org.junit.Assert.*;

import android.content.*;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;

/** Actual Media3 decoder/cache/service checks, using generated local media only. */
@RunWith(AndroidJUnit4.class)
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class MediaRuntimeTest {
  @org.junit.Rule
  public androidx.test.rule.GrantPermissionRule notifications =
      androidx.test.rule.GrantPermissionRule.grant(
          android.os.Build.VERSION.SDK_INT >= 33
              ? new String[] {"android.permission.POST_NOTIFICATIONS"}
              : new String[] {});

  interface Check {
    boolean ready() throws Exception;
  }

  static void capture(String filename) {
    try {
      android.graphics.Bitmap shot =
          InstrumentationRegistry.getInstrumentation().getUiAutomation().takeScreenshot();
      if (shot != null) {
        try (FileOutputStream out =
            new FileOutputStream(
                new File(
                    InstrumentationRegistry.getInstrumentation()
                        .getTargetContext()
                        .getExternalFilesDir(null),
                    filename))) {
          shot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out);
        } finally {
          shot.recycle();
        }
      }
    } catch (Exception ignored) {
    }
  }

  static void await(String message, Check check) throws Exception {
    long deadline = android.os.SystemClock.elapsedRealtime() + 60000;
    while (!check.ready()) {
      if (!status().isNull("error") || android.os.SystemClock.elapsedRealtime() > deadline) {
        capture("video-error.png");
        throw new AssertionError(
            message
                + ": "
                + MediaBridge.playerSnapshot
                + " Main thread: "
                + java.util.Arrays.toString(
                    android.os.Looper.getMainLooper().getThread().getStackTrace()));
      }
      Thread.sleep(100);
    }
  }

  static JSONObject status() throws Exception {
    return new JSONObject(MediaBridge.playerSnapshot);
  }

  static void command(String op) throws Exception {
    MediaBridge.execute(new JSONObject().put("op", op).toString());
  }

  @Test
  public void actualVideoPauseVolumeSwitchDownloadAndOfflinePlayback() throws Exception {
    Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
    File media = new File(context.getCacheDir(), "instrumented-fixture.mp4");
    try (InputStream input =
            InstrumentationRegistry.getInstrumentation()
                .getContext()
                .getAssets()
                .open("fixture.mp4");
        FileOutputStream output = new FileOutputStream(media)) {
      byte[] data = new byte[8192];
      int n;
      while ((n = input.read(data)) != -1) output.write(data, 0, n);
    }
    long firstGeneration;
    long cleanShutdowns = NativeBridge.uiShutdowns();
    try (ActivityScenario<RustActivity> scenario = ActivityScenario.launch(RustActivity.class)) {
      JSONObject request =
          new JSONObject()
              .put(
                  "media",
                  new JSONObject()
                      .put("url", "https://media.test/video")
                      .put("title", "Instrumented decoder")
                      .put("kind", "video"))
              .put(
                  "stream", new JSONObject().put("url", android.net.Uri.fromFile(media).toString()))
              .put("audio", false);
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
      MediaBridge.event("show_player", "");
      await(
          "Video decoder never advanced",
          () -> status().optDouble("position") > 0.2 && status().optBoolean("playing"));
      await("Video never reached the Rust surface", () -> MediaBridge.videoFrames.get() > 0);
      AudioTap.enabled = true;
      await("Decoded audio never produced visualizer levels", () -> AudioTap.levels != 0);
      android.graphics.Bitmap screenshot =
          InstrumentationRegistry.getInstrumentation().getUiAutomation().takeScreenshot();
      if (screenshot != null) {
        try (FileOutputStream out =
            new FileOutputStream(new File(context.getExternalFilesDir(null), "video-player.png"))) {
          screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out);
        } finally {
          screenshot.recycle();
        }
      }
      command("pause");
      await("Pause was not applied", () -> status().optBoolean("paused"));
      MediaBridge.execute("{\"op\":\"volume\",\"value\":0.23}");
      await("Volume did not change", () -> Math.abs(status().optDouble("volume") - 0.23) < 0.01);
      request.getJSONObject("media").put("title", "Second stream");
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
      await(
          "Stream switch failed",
          () ->
              status().optString("title").equals("Second stream")
                  && status().optDouble("position") > 0.1);
      assertEquals(0.23, status().getDouble("volume"), 0.01);
      assertTrue(status().isNull("error"));
      JSONObject preview = new JSONObject(request.toString()).put("preview", true);
      preview.getJSONObject("media").put("title", "Short muted preview");
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", preview).toString());
      await(
          "Preview did not start",
          () -> status().optBoolean("preview") && status().optBoolean("playing"));
      MediaBridge.execute("{\"op\":\"volume\",\"value\":0.8}");
      await("Preview was not muted", () -> status().optDouble("actual_volume", -1) == 0);
      assertEquals(0.23, status().getDouble("volume"), 0.01);
      await(
          "Short preview EOS failed to restore the prior media",
          () ->
              !status().optBoolean("preview")
                  && status().optString("title").equals("Second stream")
                  && status().optBoolean("playing"));
      assertEquals(0.23, status().getDouble("actual_volume"), 0.01);
      try (FixtureServer server = new FixtureServer(media)) {
        request.getJSONObject("stream").put("url", server.url());
        request.getJSONObject("media").put("url", server.url()).put("title", "Offline fixture");
        MediaBridge.execute(
            new JSONObject().put("op", "download").put("request", request).toString());
        String id = OfflineStore.id(request);
        await(
            "Real HTTP download did not finish",
            () -> {
              androidx.media3.exoplayer.offline.Download d =
                  OfflineStore.manager().getDownloadIndex().getDownload(id);
              return d != null
                  && d.state == androidx.media3.exoplayer.offline.Download.STATE_COMPLETED;
            });
        server.close();
        MediaBridge.execute(new JSONObject().put("op", "offline").put("id", id).toString());
        await(
            "Cached video could not play without its server",
            () ->
                status().optString("title").equals("Offline fixture")
                    && status().optDouble("position") > 0.2
                    && status().optBoolean("playing"));
        assertTrue(status().isNull("error"));
        MediaBridge.execute(new JSONObject().put("op", "download_remove").put("id", id).toString());
      }
      command("stop");
      firstGeneration = NativeBridge.uiGeneration();
    } finally {
      media.delete();
    }
    await("Rust UI exit hook did not complete", () -> NativeBridge.uiShutdowns() > cleanShutdowns);
    java.util.concurrent.CountDownLatch alive = new java.util.concurrent.CountDownLatch(1);
    MediaBridge.main.post(alive::countDown);
    assertTrue(
        "Activity teardown blocked the Android main looper",
        alive.await(5, java.util.concurrent.TimeUnit.SECONDS));
    try (ActivityScenario<RustActivity> reopened = ActivityScenario.launch(RustActivity.class)) {
      await(
          "Rust UI did not restart in the existing app process",
          () -> NativeBridge.uiGeneration() > firstGeneration);
    }
    await(
        "Reopened Rust UI did not clean up",
        () -> NativeBridge.uiShutdowns() >= cleanShutdowns + 2);
    java.util.concurrent.CountDownLatch closedAgain = new java.util.concurrent.CountDownLatch(1);
    MediaBridge.main.post(closedAgain::countDown);
    assertTrue(closedAgain.await(5, java.util.concurrent.TimeUnit.SECONDS));
  }

  static final class FixtureServer implements AutoCloseable {
    final ServerSocket socket;
    final byte[] bytes;
    final Thread thread;

    FixtureServer(File file) throws Exception {
      bytes = java.nio.file.Files.readAllBytes(file.toPath());
      socket = new ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"));
      thread =
          new Thread(
              () -> {
                while (!socket.isClosed()) {
                  try (Socket client = socket.accept()) {
                    BufferedReader reader =
                        new BufferedReader(
                            new InputStreamReader(
                                client.getInputStream(), StandardCharsets.US_ASCII));
                    String line = reader.readLine();
                    if (line == null) continue;
                    boolean head = line.startsWith("HEAD");
                    int start = 0;
                    while ((line = reader.readLine()) != null && !line.isEmpty()) {
                      if (line.toLowerCase(java.util.Locale.ROOT).startsWith("range: bytes="))
                        start = Integer.parseInt(line.substring(13).split("-")[0]);
                    }
                    if (start < 0 || start >= bytes.length) start = 0;
                    OutputStream out = client.getOutputStream();
                    String headers =
                        "HTTP/1.1 "
                            + (start > 0 ? "206 Partial Content" : "200 OK")
                            + "\r\n"
                            + "Content-Type: video/mp4\r\n"
                            + "Accept-Ranges: bytes\r\n"
                            + "Content-Length: "
                            + (bytes.length - start)
                            + "\r\n"
                            + (start > 0
                                ? "Content-Range: bytes "
                                    + start
                                    + "-"
                                    + (bytes.length - 1)
                                    + "/"
                                    + bytes.length
                                    + "\r\n"
                                : "")
                            + "Connection: close\r\n\r\n";
                    out.write(headers.getBytes(StandardCharsets.US_ASCII));
                    if (!head) out.write(bytes, start, bytes.length - start);
                    out.flush();
                  } catch (Exception ignored) {
                  }
                }
              },
              "local-media-fixture");
      thread.start();
    }

    String url() {
      return "http://127.0.0.1:" + socket.getLocalPort() + "/fixture.mp4";
    }

    public void close() throws Exception {
      socket.close();
      thread.join(2000);
    }
  }
}
