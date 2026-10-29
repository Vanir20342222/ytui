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
  public final org.junit.rules.ExternalResource nativeControlPolling =
      new org.junit.rules.ExternalResource() {
        private long previous;

        @Override
        protected void before() {
          androidx.test.uiautomator.Configurator config =
              androidx.test.uiautomator.Configurator.getInstance();
          previous = config.getWaitForIdleTimeout();
          // The moving time bar never stays idle for the default ten seconds;
          // a snapshot taken then observes only the already auto-hidden toolbar.
          config.setWaitForIdleTimeout(100);
        }

        @Override
        protected void after() {
          androidx.test.uiautomator.Configurator.getInstance().setWaitForIdleTimeout(previous);
        }
      };

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
        surfaceEvidence();
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

  static boolean fullscreenHasInputFocus() {
    java.util.concurrent.atomic.AtomicBoolean focused =
        new java.util.concurrent.atomic.AtomicBoolean();
    InstrumentationRegistry.getInstrumentation()
        .runOnMainSync(
            () -> {
              for (android.app.Activity activity :
                  androidx.test.runner.lifecycle.ActivityLifecycleMonitorRegistry.getInstance()
                      .getActivitiesInStage(androidx.test.runner.lifecycle.Stage.RESUMED)) {
                if (activity instanceof FullscreenActivity && activity.hasWindowFocus())
                  focused.set(true);
              }
            });
    return focused.get();
  }

  static void acknowledgeSystemFullscreenHint(androidx.test.uiautomator.UiDevice device) {
    androidx.test.uiautomator.BySelector title =
        androidx.test.uiautomator.By.res("android:id/immersive_cling_title");
    if (device.wait(androidx.test.uiautomator.Until.findObject(title), 3000) == null) return;
    androidx.test.uiautomator.UiObject2 acknowledge =
        device.findObject(androidx.test.uiautomator.By.res("android:id/ok"));
    assertNotNull("Android's fullscreen tutorial needs its acknowledgment button", acknowledge);
    acknowledge.click();
    assertTrue(
        "The system tutorial must leave the video touch surface",
        device.wait(androidx.test.uiautomator.Until.gone(title), 3000));
  }

  static void surfaceEvidence() {
    if (android.os.Build.VERSION.SDK_INT < 29) return;
    java.util.concurrent.CountDownLatch recorded = new java.util.concurrent.CountDownLatch(1);
    MediaBridge.main.post(
        () -> {
          try {
            for (android.view.View root :
                android.view.inspector.WindowInspector.getGlobalWindowViews()) inspectSurface(root);
          } finally {
            recorded.countDown();
          }
        });
    try {
      recorded.await(5, java.util.concurrent.TimeUnit.SECONDS);
    } catch (InterruptedException interrupted) {
      Thread.currentThread().interrupt();
    }
  }

  static void inspectSurface(android.view.View view) {
    if (view instanceof androidx.media3.ui.PlayerView) {
      androidx.media3.ui.PlayerView playerView = (androidx.media3.ui.PlayerView) view;
      android.view.View surface = playerView.getVideoSurfaceView();
      android.util.Log.i(
          "NativeVideoEvidence",
          "view="
              + (surface == null ? "none" : surface.getClass().getSimpleName())
              + " attached="
              + view.isAttachedToWindow()
              + " hardware="
              + view.isHardwareAccelerated()
              + " size="
              + view.getWidth()
              + "x"
              + view.getHeight()
              + " bound="
              + (playerView.getPlayer() != null));
      if (surface instanceof android.view.TextureView) {
        android.view.TextureView texture = (android.view.TextureView) surface;
        android.util.Log.i("NativeVideoEvidence", "texture_available=" + texture.isAvailable());
        android.graphics.Bitmap frame = texture.getBitmap();
        if (frame != null) {
          try (FileOutputStream output =
              new FileOutputStream(
                  new File(MediaBridge.context.getExternalFilesDir(null), "video-texture.png"))) {
            frame.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, output);
          } catch (IOException ignored) {
          } finally {
            frame.recycle();
          }
        }
      }
    }
    if (view instanceof android.view.ViewGroup) {
      android.view.ViewGroup group = (android.view.ViewGroup) view;
      for (int i = 0; i < group.getChildCount(); i++) inspectSurface(group.getChildAt(i));
    }
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
    long cleanShutdowns = RustActivity.shutdowns.get();
    try (ActivityScenario<RustActivity> scenario = RuntimeTest.launchOffline()) {
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
      long initialFrames = MediaBridge.videoFrames.get();
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
      MediaBridge.event("show_player", "");
      await(
          "Video decoder never advanced",
          () -> status().optDouble("position") > 0.2 && status().optBoolean("playing"));
      await(
          "Video decoder did not submit a native frame",
          () -> MediaBridge.videoFrames.get() > initialFrames);
      AudioTap.enabled = true;
      await("Decoded audio never produced visualizer levels", () -> AudioTap.levels != 0);
      // The decoder can start against its placeholder before Compose attaches
      // the visible view. Keep the short fixture playing until a visible frame
      // arrives; pausing before attachment can retain only a placeholder frame.
      command("repeat");
      await(
          "Decoded video was not visible in the native Android surface",
          () -> {
            android.graphics.Bitmap screenshot =
                InstrumentationRegistry.getInstrumentation().getUiAutomation().takeScreenshot();
            if (screenshot == null) return false;
            try {
              int red = 0, green = 0, blue = 0;
              for (int y = 0; y < screenshot.getHeight(); y += 3)
                for (int x = 0; x < screenshot.getWidth(); x += 3) {
                  int pixel = screenshot.getPixel(x, y),
                      r = android.graphics.Color.red(pixel),
                      g = android.graphics.Color.green(pixel),
                      b = android.graphics.Color.blue(pixel);
                  if (r > 180 && g < 90 && b < 90) red++;
                  if (g > 180 && r < 90 && b < 90) green++;
                  if (b > 180 && r < 90 && g < 90) blue++;
                }
              try (FileOutputStream out =
                  new FileOutputStream(
                      new File(context.getExternalFilesDir(null), "video-player.png"))) {
                screenshot.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out);
              }
              return red > 100 && green > 100 && blue > 100;
            } finally {
              screenshot.recycle();
            }
          });
      long fullscreenGeneration = PlaybackService.current.generation;
      long inlineFirstFrameMs = status().getLong("first_frame_ms");
      assertTrue("The decoder must report its first frame", inlineFirstFrameMs > 0);
      androidx.test.uiautomator.UiDevice device =
          androidx.test.uiautomator.UiDevice.getInstance(
              InstrumentationRegistry.getInstrumentation());
      androidx.test.uiautomator.UiObject2 fullscreen =
          device.wait(
              androidx.test.uiautomator.Until.findObject(
                  androidx.test.uiautomator.By.desc("Fullscreen video")),
              5000);
      assertNotNull("Fullscreen button must be visible", fullscreen);
      fullscreen.click();
      await(
          "Fullscreen did not enter landscape",
          () -> FullscreenActivity.visible && device.getDisplayWidth() > device.getDisplayHeight());
      acknowledgeSystemFullscreenHint(device);
      await(
          "Fullscreen must receive input after the system hint",
          MediaRuntimeTest::fullscreenHasInputFocus);
      assertEquals(
          "Fullscreen must reuse the prepared decoder",
          fullscreenGeneration,
          PlaybackService.current.generation);
      await(
          "Fullscreen must display the decoded RGB fixture, not the previous dialog",
          () -> {
            android.graphics.Bitmap frame =
                InstrumentationRegistry.getInstrumentation().getUiAutomation().takeScreenshot();
            if (frame == null) return false;
            try {
              if (frame.getWidth() <= frame.getHeight()) return false;
              int red = 0, green = 0, blue = 0;
              for (int y = 0; y < frame.getHeight(); y += 8)
                for (int x = 0; x < frame.getWidth(); x += 8) {
                  int pixel = frame.getPixel(x, y);
                  int r = android.graphics.Color.red(pixel);
                  int g = android.graphics.Color.green(pixel);
                  int b = android.graphics.Color.blue(pixel);
                  if (r > 180 && g < 90 && b < 90) red++;
                  if (g > 180 && r < 90 && b < 90) green++;
                  if (b > 180 && r < 90 && g < 90) blue++;
                }
              if (red < 100 || green < 100 || blue < 100) return false;
              try (FileOutputStream out =
                  new FileOutputStream(
                      new File(context.getExternalFilesDir(null), "native-video-fullscreen.png"))) {
                frame.compress(android.graphics.Bitmap.CompressFormat.PNG, 100, out);
              }
              return true;
            } finally {
              frame.recycle();
            }
          });
      androidx.test.uiautomator.UiObject2 exitFullscreen =
          device.findObject(
              androidx.test.uiautomator.By.res(
                  java.util.regex.Pattern.compile(".*:id/exo_(minimal_)?fullscreen")));
      if (exitFullscreen == null) {
        device.click(device.getDisplayWidth() / 4, device.getDisplayHeight() / 3);
        exitFullscreen =
            device.wait(
                androidx.test.uiautomator.Until.findObject(
                    androidx.test.uiautomator.By.res(
                        java.util.regex.Pattern.compile(".*:id/exo_(minimal_)?fullscreen"))),
                5000);
      }
      capture("native-video-controls.png");
      device.dumpWindowHierarchy(
          new File(context.getExternalFilesDir(null), "native-video-controls.xml"));
      assertNotNull("The native fullscreen exit button must be reachable", exitFullscreen);
      exitFullscreen.click();
      await("Fullscreen did not return to the focused player", () -> !FullscreenActivity.visible);
      assertEquals(fullscreenGeneration, PlaybackService.current.generation);
      assertEquals(
          "Surface switches must not overwrite startup latency",
          inlineFirstFrameMs,
          status().getLong("first_frame_ms"));
      fullscreen =
          device.wait(
              androidx.test.uiautomator.Until.findObject(
                  androidx.test.uiautomator.By.desc("Fullscreen video")),
              5000);
      assertNotNull("Focused player must return after leaving fullscreen", fullscreen);
      fullscreen.click();
      await("Fullscreen must reopen", () -> FullscreenActivity.visible);
      acknowledgeSystemFullscreenHint(device);
      await("Reopened fullscreen must receive input", MediaRuntimeTest::fullscreenHasInputFocus);
      InstrumentationRegistry.getInstrumentation().waitForIdleSync();
      device.pressBack();
      await("Android Back must exit fullscreen", () -> !FullscreenActivity.visible);
      assertEquals(fullscreenGeneration, PlaybackService.current.generation);
      command("pause");
      await("Pause was not applied", () -> status().optBoolean("paused"));
      command("repeat");
      double position = status().getDouble("position");
      java.util.concurrent.CountDownLatch replaced = new java.util.concurrent.CountDownLatch(1);
      java.util.concurrent.atomic.AtomicReference<Exception> replaceError =
          new java.util.concurrent.atomic.AtomicReference<>();
      MediaBridge.main.post(
          () -> {
            try {
              PlaybackService service = PlaybackService.current;
              service.replaceQuality(
                  service.currentRequest, request.optJSONObject("stream"), "720");
            } catch (Exception error) {
              replaceError.set(error);
            } finally {
              replaced.countDown();
            }
          });
      assertTrue(replaced.await(5, java.util.concurrent.TimeUnit.SECONDS));
      if (replaceError.get() != null) throw replaceError.get();
      await(
          "Quality switch did not keep pause and position",
          () ->
              status().optString("quality").equals("720")
                  && status().optBoolean("paused")
                  && Math.abs(status().optDouble("position") - position) < 0.5);
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
      firstGeneration = RustActivity.generations.get();
    } finally {
      MediaBridge.main.post(
          () -> {
            if (PlaybackService.current != null)
              PlaybackService.current.player.setRepeatMode(
                  androidx.media3.common.Player.REPEAT_MODE_OFF);
          });
      command("stop");
      AudioTap.enabled = false;
      media.delete();
    }
    await("Android UI did not close", () -> RustActivity.shutdowns.get() > cleanShutdowns);
    java.util.concurrent.CountDownLatch alive = new java.util.concurrent.CountDownLatch(1);
    MediaBridge.main.post(alive::countDown);
    assertTrue(
        "Activity teardown blocked the Android main looper",
        alive.await(5, java.util.concurrent.TimeUnit.SECONDS));
    try (ActivityScenario<RustActivity> reopened = RuntimeTest.launchOffline()) {
      await(
          "Android UI did not restart in the existing app process",
          () -> RustActivity.generations.get() > firstGeneration);
    }
    await(
        "Reopened Android UI did not clean up",
        () -> RustActivity.shutdowns.get() >= cleanShutdowns + 2);
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
