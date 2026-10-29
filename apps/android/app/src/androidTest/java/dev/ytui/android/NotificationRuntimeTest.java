package dev.ytui.android;

import static org.junit.Assert.*;

import android.content.Context;
import android.os.SystemClock;
import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import org.json.*;
import org.junit.*;
import org.junit.runner.RunWith;

/**
 * The real notification/session handoff must not turn a new start into a delayed System UI Stop.
 */
@RunWith(AndroidJUnit4.class)
public final class NotificationRuntimeTest {
  @Rule
  public androidx.test.rule.GrantPermissionRule notifications =
      androidx.test.rule.GrantPermissionRule.grant(
          android.os.Build.VERSION.SDK_INT >= 33
              ? new String[] {"android.permission.POST_NOTIFICATIONS"}
              : new String[] {});

  @Test
  public void consecutiveServiceSessionsRemainPlayingAfterNotificationReplacement()
      throws Exception {
    Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
    File file = new File(context.getCacheDir(), "notification-fixture.mp4");
    try (InputStream input =
            InstrumentationRegistry.getInstrumentation()
                .getContext()
                .getAssets()
                .open("fixture.mp4");
        FileOutputStream output = new FileOutputStream(file)) {
      byte[] bytes = new byte[8192];
      int count;
      while ((count = input.read(bytes)) != -1) output.write(bytes, 0, count);
    }
    try (ActivityScenario<RustActivity> activity = RuntimeTest.launchOffline()) {
      MediaRuntimeTest.command("stop");
      MediaRuntimeTest.await(
          "Old playback service did not finish", () -> PlaybackService.current == null);
      for (int index = 0; index < 6; index++) {
        String url = "https://media.test/notification/" + index;
        JSONObject request =
            new JSONObject()
                .put("audio", index % 2 == 0)
                .put("local", true)
                .put(
                    "media",
                    new JSONObject()
                        .put("url", url)
                        .put("title", "Notification fixture")
                        .put("kind", index % 2 == 0 ? "song" : "video"))
                .put(
                    "stream",
                    new JSONObject().put("url", android.net.Uri.fromFile(file).toString()));
        MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
        MediaRuntimeTest.await(
            "New session never started",
            () -> {
              JSONObject state = MediaRuntimeTest.status();
              JSONObject media = state.optJSONObject("media");
              return media != null
                  && url.equals(media.optString("url"))
                  && state.optBoolean("playing");
            });
        SystemClock.sleep(1800);
        assertNotNull("System UI unexpectedly destroyed the new service", PlaybackService.current);
        assertTrue(
            "New playback was stopped after notification replacement",
            MediaRuntimeTest.status().optBoolean("playing"));
        android.app.NotificationManager manager =
            context.getSystemService(android.app.NotificationManager.class);
        boolean posted = false;
        for (android.service.notification.StatusBarNotification item :
            manager.getActiveNotifications()) {
          if (item.getId() == NotificationIds.PLAYBACK
              && PlaybackService.current
                  .session
                  .getPlatformToken()
                  .equals(
                      item.getNotification()
                          .extras
                          .getParcelable(android.app.Notification.EXTRA_MEDIA_SESSION)))
            posted = true;
        }
        assertTrue("Active playback needs the current session notification", posted);
        MediaRuntimeTest.command("stop");
        MediaRuntimeTest.await(
            "Stopped service did not finish", () -> PlaybackService.current == null);
      }
    } finally {
      MediaRuntimeTest.command("stop");
    }
  }
}
