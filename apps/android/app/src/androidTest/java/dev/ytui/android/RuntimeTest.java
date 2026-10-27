package dev.ytui.android;

import static org.junit.Assert.*;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import java.nio.charset.StandardCharsets;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public final class RuntimeTest {
  @Test
  public void rustJniLoadsAndReturnsBoundedContracts() throws Exception {
    JSONObject settings = new JSONObject(NativeBridge.request("{\"op\":\"settings\"}"));
    assertTrue(settings.optBoolean("ok"));
    assertTrue(settings.getJSONObject("data").has("music_player_style"));
    JSONObject unknown = new JSONObject(NativeBridge.request("{\"op\":\"not_an_operation\"}"));
    assertFalse(unknown.getBoolean("ok"));
    assertTrue(unknown.getString("error").length() < 1000);
    JSONObject malformed = new JSONObject(NativeBridge.request("{"));
    assertFalse(malformed.getBoolean("ok"));
    java.nio.ByteBuffer silence = java.nio.ByteBuffer.allocateDirect(32);
    assertEquals(0, NativeBridge.meter(silence, 0, 32, 2));
    assertEquals(0, NativeBridge.meter(silence, 30, 32, 2));
  }

  @Test
  public void emptyAudioProcessorInputDoesNotCopyTheSharedEosBuffer() throws Exception {
    AudioTap tap = new AudioTap();
    tap.configure(
        new androidx.media3.common.audio.AudioProcessor.AudioFormat(
            44100, 1, androidx.media3.common.C.ENCODING_PCM_16BIT));
    tap.flush();
    tap.queueInput(androidx.media3.common.audio.AudioProcessor.EMPTY_BUFFER);
    assertFalse(tap.getOutput().hasRemaining());
    java.nio.ByteBuffer samples =
        java.nio.ByteBuffer.allocateDirect(4).order(java.nio.ByteOrder.LITTLE_ENDIAN);
    samples.putShort((short) 12000).putShort((short) -12000).flip();
    AudioTap.enabled = true;
    tap.queueInput(samples);
    assertTrue(AudioTap.levels != 0);
    assertEquals(4, tap.getOutput().remaining());
    tap.queueInput(androidx.media3.common.audio.AudioProcessor.EMPTY_BUFFER);
    tap.reset();
  }

  @Test
  public void isolatedJavascriptHasNoBrowserNetworkOrCookieApis() throws Exception {
    String value =
        ScriptRuntime.evaluate(
            "console.log(JSON.stringify({fetch:typeof fetch,document:typeof document,window:typeof"
                + " window}));");
    JSONObject capabilities = new JSONObject(value);
    assertEquals("undefined", capabilities.getString("fetch"));
    assertEquals("undefined", capabilities.getString("document"));
    assertEquals("undefined", capabilities.getString("window"));
  }

  @Test
  public void youtubeNAndSignatureVectorsPassInAndroidIsolate() throws Exception {
    JSONObject expected = new JSONObject(asset("expected.json"));
    JSONArray wanted = expected.getJSONArray("requests"), requests = new JSONArray();
    for (int i = 0; i < wanted.length(); i++) {
      JSONObject row = wanted.getJSONObject(i);
      JSONArray challenges = new JSONArray();
      java.util.Iterator<String> names = row.getJSONObject("challenges").keys();
      while (names.hasNext()) challenges.put(names.next());
      requests.put(
          new JSONObject().put("type", row.getString("type")).put("challenges", challenges));
    }
    JSONObject payload =
        new JSONObject()
            .put("type", "player")
            .put("player", asset(expected.getString("player_file")))
            .put("requests", requests);
    String script =
        asset("yt.solver.lib.js")
            + "\nObject.assign(globalThis,lib);\n"
            + asset("yt.solver.core.js")
            + "\nconsole.log(JSON.stringify(jsc("
            + payload
            + ")));";
    JSONObject result = new JSONObject(ScriptRuntime.evaluate(script));
    assertEquals("result", result.getString("type"));
    JSONArray responses = result.getJSONArray("responses");
    assertEquals(wanted.length(), responses.length());
    for (int i = 0; i < wanted.length(); i++) {
      JSONObject want = wanted.getJSONObject(i), response = responses.getJSONObject(i);
      assertEquals(want.getString("type"), response.getString("type"));
      JSONObject values = want.getJSONObject("challenges"), actual = response.getJSONObject("data");
      java.util.Iterator<String> names = values.keys();
      while (names.hasNext()) {
        String key = names.next();
        assertEquals(values.getString(key), actual.getString(key));
      }
    }
  }

  private String asset(String name) throws Exception {
    try (InputStream input =
        InstrumentationRegistry.getInstrumentation().getContext().getAssets().open(name)) {
      ByteArrayOutputStream out = new ByteArrayOutputStream();
      byte[] buffer = new byte[8192];
      int n;
      while ((n = input.read(buffer)) != -1) out.write(buffer, 0, n);
      return out.toString(StandardCharsets.UTF_8.name());
    }
  }
}
