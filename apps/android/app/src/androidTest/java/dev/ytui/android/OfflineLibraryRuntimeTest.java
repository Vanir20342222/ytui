package dev.ytui.android;

import static org.junit.Assert.*;

import android.content.*;
import android.net.Uri;
import android.provider.DocumentsContract;
import androidx.media3.exoplayer.offline.*;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.uiautomator.*;
import java.nio.charset.StandardCharsets;
import java.util.*;
import java.util.concurrent.TimeUnit;
import org.json.*;
import org.junit.*;
import org.junit.runner.RunWith;

/** Real document-tree grant, durable folder export, legacy migration and cache-free local EOF. */
@RunWith(AndroidJUnit4.class)
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class OfflineLibraryRuntimeTest {
  @Rule
  public androidx.test.rule.GrantPermissionRule notifications =
      androidx.test.rule.GrantPermissionRule.grant(
          android.os.Build.VERSION.SDK_INT >= 33
              ? new String[] {"android.permission.POST_NOTIFICATIONS"}
              : new String[] {});

  @Test
  public void folderPickerExportsLegacyAndNewMediaAndLocalPlaylistAdvancesWithoutItsServer()
      throws Exception {
    Context ctx = InstrumentationRegistry.getInstrumentation().getTargetContext();
    UiDevice device = UiDevice.getInstance(InstrumentationRegistry.getInstrumentation());
    String folderName = "ytui-fixture-" + UUID.randomUUID();
    device.executeShellCommand("mkdir -p /sdcard/Music/" + folderName);
    Uri tree =
        DocumentsContract.buildTreeDocumentUri(
            "com.android.externalstorage.documents", "primary:Music/" + folderName);
    String oldFolder = StorageLocations.prefs().getString("music", "");
    boolean auto = MobileSettings.enabled("auto_save_folders");
    JSONObject config = PlaybackService.rpc(new JSONObject().put("op", "settings"));
    String firstId = null, secondId = null;
    try (ActivityScenario<RustActivity> scenario = RuntimeTest.launchOffline();
        MediaRuntimeTest.FixtureServer server =
            new MediaRuntimeTest.FixtureServer(ExportRuntimeTest.asset(ctx, "fixture-audio.m4a"))) {
      MobileSettings.prefs().edit().putBoolean("auto_save_folders", false).commit();
      StorageLocations.prefs().edit().putString("music", tree.toString()).commit();
      assertEquals(
          DocumentsContract.buildDocumentUri(
              tree.getAuthority(), DocumentsContract.getTreeDocumentId(tree)),
          StorageLocations.initialDocument(true));
      scenario.onActivity(activity -> activity.pickFolder(true));
      assertNotNull(
          "Folder picker opened the wrong directory",
          device.wait(Until.findObject(By.text(folderName)), 10000));
      UiObject2 use =
          device.wait(
              Until.findObject(By.text(java.util.regex.Pattern.compile("(?i)use this folder"))),
              10000);
      assertNotNull("System folder picker did not open at the requested folder", use);
      assertTrue("The chosen folder must be writable", use.isEnabled());
      use.click();
      UiObject2 allow =
          device.wait(
              Until.findObject(By.text(java.util.regex.Pattern.compile("(?i)allow"))), 3000);
      if (allow != null) allow.click();
      MediaRuntimeTest.await(
          "Document-tree read/write permission was not persisted",
          () -> StorageLocations.granted(tree));
      JSONObject first =
          new JSONObject()
              .put(
                  "media",
                  new JSONObject()
                      .put("url", server.url() + "?first")
                      .put("title", "Offline first")
                      .put("kind", "song"))
              .put("audio", true)
              .put(
                  "stream",
                  new JSONObject().put("url", server.url()).put("format_id", "audio-fixture"));
      JSONObject second =
          new JSONObject()
              .put(
                  "media",
                  new JSONObject()
                      .put("url", server.url() + "?second")
                      .put("title", "Offline second")
                      .put("kind", "song"))
              .put("audio", true)
              .put(
                  "stream",
                  new JSONObject().put("url", server.url()).put("format_id", "audio-fixture"));
      JSONObject lease =
          new JSONObject()
              .put("format_id", "140")
              .put("url", "https://cdn.test/a?clen=4096&lmt=12&sig=old");
      String leaseKey = OfflineStore.cacheKey("id", lease, false);
      lease.put("url", "https://cdn.test/a?clen=4096&lmt=12&sig=new");
      assertEquals(
          "Renewing a lease must retain progressive cached bytes",
          leaseKey,
          OfflineStore.cacheKey("id", lease, false));
      lease.put("format_id", "251");
      assertNotEquals(leaseKey, OfflineStore.cacheKey("id", lease, false));
      firstId = OfflineStore.hashId(first.getJSONObject("media").getString("url"));
      secondId = OfflineStore.id(second);
      assertNotEquals(
          "Audio/video offline identities must differ",
          OfflineStore.id(second),
          OfflineStore.id(new JSONObject(second.toString()).put("audio", false)));
      DownloadService.sendAddDownload(
          ctx,
          OfflineService.class,
          new DownloadRequest.Builder(firstId, Uri.parse(server.url()))
              .setData(first.toString().getBytes(StandardCharsets.UTF_8))
              .build(),
          true);
      MediaBridge.execute(new JSONObject().put("op", "download").put("request", second).toString());
      final String a = firstId, b = secondId;
      MediaRuntimeTest.await(
          "Offline copies did not finish",
          () -> ExportRuntimeTest.complete(a) && ExportRuntimeTest.complete(b));
      assertNotNull(
          "Legacy URL-only cache must stay discoverable",
          OfflineStore.find(first.getJSONObject("media"), true));
      MobileSettings.prefs().edit().putBoolean("auto_save_folders", true).commit();
      OfflineExport.enqueueAuto(a);
      OfflineExport.enqueueAuto(b);
      MediaRuntimeTest.await(
          "Completed media was not saved to the granted folder",
          () -> StorageLocations.record(a) != null && StorageLocations.record(b) != null);
      JSONObject savedA = StorageLocations.request(a), savedB = StorageLocations.request(b);
      assertEquals(
          "content", Uri.parse(savedA.getJSONObject("stream").getString("url")).getScheme());
      assertTrue(savedA.optBoolean("local"));
      DownloadService.sendRemoveDownload(ctx, OfflineService.class, a, false);
      DownloadService.sendRemoveDownload(ctx, OfflineService.class, b, false);
      MediaRuntimeTest.await(
          "Cache-only copies were not removed",
          () ->
              OfflineStore.manager().getDownloadIndex().getDownload(a) == null
                  && OfflineStore.manager().getDownloadIndex().getDownload(b) == null);
      server.close();
      MediaBridge.playbackStore
          .submit(
              () -> {
                PlaybackService.rpcArray(new JSONObject().put("op", "history_clear"));
                return null;
              })
          .get(5, TimeUnit.SECONDS);
      PlaybackNavigationTest.clearQueue();
      JSONArray items =
          new JSONArray().put(savedA.getJSONObject("media")).put(savedB.getJSONObject("media"));
      String token = "offline-folder-fixture";
      JSONObject descriptor =
          new JSONObject()
              .put("op", "offline_context")
              .put("context_id", token)
              .put("source_url", savedA.getJSONObject("media").getString("url"))
              .put("items", items);
      MediaBridge.execute(
          new JSONObject()
              .put("op", "offline")
              .put("id", a)
              .put("context_id", token)
              .put("context", descriptor)
              .toString());
      PlaybackNavigationTest.playing("Offline first");
      assertTrue(
          "Local file playback must be marked offline",
          MediaRuntimeTest.status().optBoolean("offline"));
      PlaybackNavigationTest.playing("Offline second");
      assertTrue(MediaRuntimeTest.status().optBoolean("offline"));
      assertNull(
          "Folder playback must not re-create a network download",
          OfflineStore.manager().getDownloadIndex().getDownload(a));
    } finally {
      MediaRuntimeTest.command("stop");
      for (String id : new String[] {firstId, secondId})
        if (id != null) {
          DownloadService.sendRemoveDownload(ctx, OfflineService.class, id, false);
          StorageLocations.remove(id);
        }
      StorageLocations.prefs().edit().putString("music", oldFolder).commit();
      MobileSettings.prefs().edit().putBoolean("auto_save_folders", auto).commit();
      try {
        ctx.getContentResolver()
            .releasePersistableUriPermission(
                tree,
                Intent.FLAG_GRANT_READ_URI_PERMISSION | Intent.FLAG_GRANT_WRITE_URI_PERMISSION);
      } catch (Exception ignored) {
      }
      MediaBridge.playbackStore.submit(() -> null).get(5, TimeUnit.SECONDS);
      PlaybackService.rpc(new JSONObject().put("op", "save_settings").put("config", config));
    }
  }
}
