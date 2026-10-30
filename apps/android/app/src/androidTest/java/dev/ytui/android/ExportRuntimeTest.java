package dev.ytui.android;

import static org.junit.Assert.*;

import android.content.Context;
import android.media.*;
import android.net.Uri;
import androidx.media3.exoplayer.offline.Download;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import java.util.*;
import org.json.*;
import org.junit.*;
import org.junit.runner.RunWith;

/** Export actual completed caches with both fixture servers closed, then decode the result. */
@RunWith(AndroidJUnit4.class)
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class ExportRuntimeTest {
  @Rule
  public androidx.test.rule.GrantPermissionRule notifications =
      androidx.test.rule.GrantPermissionRule.grant(
          android.os.Build.VERSION.SDK_INT >= 33
              ? new String[] {"android.permission.POST_NOTIFICATIONS"}
              : new String[] {});

  static File asset(Context context, String name) throws Exception {
    File file = new File(context.getCacheDir(), name);
    try (InputStream input =
            InstrumentationRegistry.getInstrumentation().getContext().getAssets().open(name);
        OutputStream output = new FileOutputStream(file)) {
      byte[] buffer = new byte[8192];
      int count;
      while ((count = input.read(buffer)) != -1) output.write(buffer, 0, count);
    }
    return file;
  }

  static boolean complete(String id) throws Exception {
    Download item = OfflineStore.manager().getDownloadIndex().getDownload(id);
    return item != null && item.state == Download.STATE_COMPLETED;
  }

  @Test
  public void separateTracksExportAndAudioCopiesWithoutNetworkOrEncoding() throws Exception {
    Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
    File video = asset(context, "fixture-video.mp4"), audio = asset(context, "fixture-audio.m4a");
    File exported = new File(context.getCacheDir(), "exported-paired.mp4"),
        song = new File(context.getCacheDir(), "exported-audio.m4a");
    File cancelled = new File(context.getCacheDir(), "cancelled-export.mp4");
    cancelled.delete();
    String videoId = null, audioId = null;
    try (ActivityScenario<RustActivity> scenario = RuntimeTest.launchOffline();
        LoopbackDownloads loopback = LoopbackDownloads.open();
        MediaRuntimeTest.FixtureServer videos = new MediaRuntimeTest.FixtureServer(video);
        MediaRuntimeTest.FixtureServer audios = new MediaRuntimeTest.FixtureServer(audio)) {
      JSONObject paired =
          new JSONObject()
              .put(
                  "media",
                  new JSONObject()
                      .put("url", videos.url())
                      .put("title", "Paired export")
                      .put("kind", "video"))
              .put(
                  "stream",
                  new JSONObject().put("url", videos.url()).put("audio_url", audios.url()))
              .put("audio", false);
      JSONObject uncachedCaptions =
          new JSONObject()
              .put(
                  "captions",
                  new JSONArray()
                      .put(
                          new JSONObject()
                              .put("url", "https://captions.test/uncached.vtt")
                              .put("label", "en")))
              .put(
                  "formats",
                  new JSONArray().put(new JSONObject().put("unused", "x".repeat(128 * 1024))));
      paired.put("quality", "480");
      paired.getJSONObject("stream").put("catalog", uncachedCaptions);
      JSONObject audioRequest =
          new JSONObject()
              .put(
                  "media",
                  new JSONObject()
                      .put("url", audios.url())
                      .put("title", "Audio export")
                      .put("kind", "song"))
              .put("stream", new JSONObject().put("url", audios.url()))
              .put("audio", true);
      videoId = OfflineStore.id(paired);
      audioId = OfflineStore.id(audioRequest);
      final String vid = videoId, aud = audioId;
      MediaBridge.execute(new JSONObject().put("op", "download").put("request", paired).toString());
      MediaBridge.execute(
          new JSONObject().put("op", "download").put("request", audioRequest).toString());
      MediaRuntimeTest.await(
          "Separate fixture downloads did not complete",
          () -> complete(vid) && complete(vid + "-audio") && complete(aud));
      videos.close();
      audios.close();
      Download stored = OfflineStore.manager().getDownloadIndex().getDownload(videoId);
      assertTrue(
          "Unused format catalogs should not inflate download metadata",
          stored.request.data.length < 4096);
      JSONObject storedRequest =
          new JSONObject(new String(stored.request.data, java.nio.charset.StandardCharsets.UTF_8));
      assertEquals("480", storedRequest.getString("quality"));
      assertFalse(storedRequest.getJSONObject("stream").has("catalog"));
      // Existing downloads may still contain uncached caption URLs. They must
      // not break cache-only playback or trigger an upstream caption request.
      JSONObject legacyOffline = OfflineStore.offlineRequest(videoId);
      legacyOffline.getJSONObject("stream").put("catalog", uncachedCaptions);
      MediaBridge.execute(
          new JSONObject().put("op", "play").put("request", legacyOffline).toString());
      MediaRuntimeTest.await(
          "Uncached legacy captions broke offline playback",
          () ->
              MediaRuntimeTest.status().optString("title").equals("Paired export")
                  && MediaRuntimeTest.status().optBoolean("playing")
                  && MediaRuntimeTest.status().optDouble("position") > 0.2);
      assertTrue(OfflineExport.inspect(videoId).container >= 0);
      OfflineExport.write(videoId, Uri.fromFile(exported), () -> false);
      MediaExtractor extractor = new MediaExtractor();
      try {
        extractor.setDataSource(exported.getAbsolutePath());
        Set<String> mimes = new HashSet<>();
        for (int i = 0; i < extractor.getTrackCount(); i++)
          mimes.add(extractor.getTrackFormat(i).getString(MediaFormat.KEY_MIME));
        assertTrue(mimes.contains("video/avc"));
        assertTrue(mimes.contains("audio/mp4a-latm"));
      } finally {
        extractor.release();
      }
      assertEquals("m4a", OfflineExport.inspect(audioId).extension);
      java.util.UUID exportJob = OfflineExport.enqueue(audioId, Uri.fromFile(song));
      MediaRuntimeTest.await(
          "Foreground audio export did not finish",
          () ->
              "Export complete"
                  .equals(
                      context
                          .getSharedPreferences("exports", Context.MODE_PRIVATE)
                          .getString(exportJob.toString(), "")));
      OfflineStore.remove("export:" + exportJob);
      assertFalse(
          "Completing an export must not destroy the active media session",
          MediaRuntimeTest.status().optString("title").isEmpty());
      for (android.service.notification.StatusBarNotification active :
          context
              .getSystemService(android.app.NotificationManager.class)
              .getActiveNotifications()) {
        if (active.getId() == NotificationIds.PLAYBACK)
          assertEquals("ytui-playback", active.getNotification().getChannelId());
      }
      assertArrayEquals(
          java.nio.file.Files.readAllBytes(audio.toPath()),
          java.nio.file.Files.readAllBytes(song.toPath()));
      try {
        OfflineExport.write(videoId, Uri.fromFile(cancelled), () -> true);
        fail("Cancelled export succeeded");
      } catch (InterruptedIOException expected) {
        assertFalse(cancelled.exists());
      }
      JSONObject playback =
          new JSONObject()
              .put(
                  "media",
                  new JSONObject()
                      .put("url", "https://media.test/export")
                      .put("title", "Exported media")
                      .put("kind", "video"))
              .put("stream", new JSONObject().put("url", Uri.fromFile(exported).toString()));
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", playback).toString());
      MediaRuntimeTest.await(
          "Exported file did not actually decode",
          () ->
              MediaRuntimeTest.status().optString("title").equals("Exported media")
                  && MediaRuntimeTest.status().optDouble("position") > 0.5
                  && MediaRuntimeTest.status().optBoolean("playing"));
      AudioTap.levels = 0;
      AudioTap.enabled = true;
      MediaRuntimeTest.await("Exported audio produced no decoded PCM", () -> AudioTap.levels != 0);
      assertTrue(MediaRuntimeTest.status().isNull("error"));
      MediaRuntimeTest.command("stop");
    } finally {
      AudioTap.enabled = false;
      for (String id : new String[] {videoId, audioId}) if (id != null) OfflineStore.remove(id);
      for (File file : new File[] {video, audio, exported, song, cancelled}) file.delete();
    }
  }
}
