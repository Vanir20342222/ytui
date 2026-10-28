package dev.ytui.android;

import static org.junit.Assert.*;

import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.json.*;
import org.junit.*;
import org.junit.runner.RunWith;

/** Explicit public-only acceptance; never imports an account or prints a stream URL. */
@RunWith(AndroidJUnit4.class)
public final class PublicYoutubeTest {
  @org.junit.Rule
  public androidx.test.rule.GrantPermissionRule notifications =
      androidx.test.rule.GrantPermissionRule.grant(
          android.os.Build.VERSION.SDK_INT >= 33
              ? new String[] {"android.permission.POST_NOTIFICATIONS"}
              : new String[] {});

  @Test
  public void nativePublicYoutubeActuallyDecodesWhenExplicitlyRequested() throws Exception {
    String url = InstrumentationRegistry.getArguments().getString("youtubeUrl", "");
    Assume.assumeTrue("Public network check is opt-in", !url.isEmpty());
    android.net.Uri parsed = android.net.Uri.parse(url);
    String host = parsed.getHost();
    assertTrue(
        "Use a public YouTube HTTP(S) URL",
        ("http".equals(parsed.getScheme()) || "https".equals(parsed.getScheme()))
            && host != null
            && (host.equals("youtu.be")
                || host.equals("youtube.com")
                || host.endsWith(".youtube.com")));
    try (ActivityScenario<RustActivity> activity = RuntimeTest.launchOffline()) {
      JSONObject media =
          new JSONObject()
              .put("url", url)
              .put("title", "Public YouTube acceptance")
              .put("kind", "video");
      JSONObject resolved =
          new JSONObject(
              NativeBridge.request(
                  new JSONObject()
                      .put("op", "resolve")
                      .put("media", media)
                      .put("audio", false)
                      .put("quality", "720")
                      .toString()));
      assertTrue(
          "Native public extraction: " + resolved.optString("error"), resolved.optBoolean("ok"));
      JSONObject request =
          new JSONObject()
              .put("media", media)
              .put("stream", resolved.getJSONObject("data"))
              .put("audio", false);
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
      MediaBridge.event("show_player", "");
      MediaRuntimeTest.await(
          "Public YouTube playback did not decode three seconds",
          () ->
              MediaRuntimeTest.status().optDouble("position") > 3
                  && MediaRuntimeTest.status().optBoolean("playing"));
      assertTrue(MediaRuntimeTest.status().isNull("error"));
      MediaRuntimeTest.command("stop");
    }
  }
}
