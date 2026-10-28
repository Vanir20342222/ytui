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
  static androidx.test.core.app.ActivityScenario<RustActivity> launchOffline() {
    android.content.Intent intent =
        new android.content.Intent(
                InstrumentationRegistry.getInstrumentation().getTargetContext(), RustActivity.class)
            .putExtra("ytui.offline-ui", true);
    return androidx.test.core.app.ActivityScenario.launch(intent);
  }

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
  public void androidKeystoreUsesRandomAuthenticatedEnvelopes() throws Exception {
    String text = "{\"fixture\":\"Grüße 🎵\"}";
    String first = AccountCipher.seal(text), second = AccountCipher.seal(text);
    assertNotEquals(first, second);
    assertEquals(text, AccountCipher.open(first));
    assertEquals(text, AccountCipher.open(second));
    int split = first.indexOf(':');
    byte[] bytes =
        android.util.Base64.decode(first.substring(split + 1), android.util.Base64.NO_WRAP);
    bytes[bytes.length - 1] ^= 1;
    String changed =
        first.substring(0, split + 1)
            + android.util.Base64.encodeToString(bytes, android.util.Base64.NO_WRAP);
    assertThrows(Exception.class, () -> AccountCipher.open(changed));
    assertThrows(Exception.class, () -> AccountCipher.open("unknown-envelope"));
    assertThrows(Exception.class, () -> AccountCipher.seal("x".repeat(2 * 1024 * 1024 + 1)));
  }

  @Test
  public void rustAccountImportPersistsOnlyCiphertextAndDisconnectRemovesIt() throws Exception {
    android.content.Context context =
        InstrumentationRegistry.getInstrumentation().getTargetContext();
    File plain = new File(context.getFilesDir(), "account.json"),
        encrypted = new File(context.getFilesDir(), "account.enc");
    org.junit.Assume.assumeFalse(
        "Account test requires a disposable disconnected install",
        plain.exists()
            || encrypted.exists()
            || PlaybackService.rpc(new JSONObject().put("op", "account")).optBoolean("connected"));
    try {
      String document =
          "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSAPISID\tsynthetic-only\n";
      PlaybackService.rpc(new JSONObject().put("op", "import_cookie_file").put("text", document));
      assertTrue(
          PlaybackService.rpc(new JSONObject().put("op", "account")).getBoolean("connected"));
      assertFalse(plain.exists());
      assertTrue(encrypted.isFile());
      String envelope =
          new String(java.nio.file.Files.readAllBytes(encrypted.toPath()), StandardCharsets.UTF_8);
      assertFalse(envelope.contains("synthetic-only"));
      JSONObject decoded = new JSONObject(AccountCipher.open(envelope));
      assertEquals(
          "synthetic-only", decoded.getJSONArray("cookies").getJSONObject(0).getString("value"));
      // Exercise real Rust migration using only this disposable synthetic account.
      java.nio.file.Files.write(
          plain.toPath(), decoded.toString().getBytes(StandardCharsets.UTF_8));
      assertTrue(encrypted.delete());
      assertTrue(
          PlaybackService.rpc(new JSONObject().put("op", "reload_account"))
              .getBoolean("connected"));
      assertFalse(plain.exists());
      assertTrue(encrypted.isFile());
      String migrated =
          new String(java.nio.file.Files.readAllBytes(encrypted.toPath()), StandardCharsets.UTF_8);
      assertEquals(
          "synthetic-only",
          new JSONObject(AccountCipher.open(migrated))
              .getJSONArray("cookies")
              .getJSONObject(0)
              .getString("value"));
      // A damaged envelope clears active auth and retains the file for recovery.
      byte[] corrupt = migrated.getBytes(StandardCharsets.UTF_8);
      corrupt[corrupt.length - 6] ^= 1;
      java.nio.file.Files.write(encrypted.toPath(), corrupt);
      JSONObject unavailable = PlaybackService.rpc(new JSONObject().put("op", "reload_account"));
      assertFalse(unavailable.getBoolean("connected"));
      assertFalse(unavailable.toString().contains("synthetic-only"));
      assertTrue(encrypted.isFile());
      java.nio.file.Files.write(encrypted.toPath(), migrated.getBytes(StandardCharsets.UTF_8));
      assertTrue(
          PlaybackService.rpc(new JSONObject().put("op", "reload_account"))
              .getBoolean("connected"));
    } finally {
      PlaybackService.rpc(new JSONObject().put("op", "disconnect"));
    }
    assertFalse(plain.exists());
    assertFalse(encrypted.exists());
  }

  @Test
  public void localLyricJniPreservesTimingsAndRejectsAnInvalidEdit() throws Exception {
    String url = "https://media.test/android-lyrics";
    try {
      JSONObject document =
          new JSONObject()
              .put("title", "Grüße 🎵")
              .put("artist", "Artist")
              .put("text", "[00:01.234]First\n[00:03.987]Second")
              .put("offset_ms", 500);
      JSONObject saved =
          PlaybackService.rpc(
              new JSONObject().put("op", "lyrics_save").put("url", url).put("document", document));
      assertEquals(
          1234,
          saved.getJSONObject("track").getJSONArray("lines").getJSONObject(0).getLong("at_ms"));
      JSONObject invalid = new JSONObject(document.toString()).put("offset_ms", 120001);
      JSONObject bad =
          new JSONObject(
              NativeBridge.request(
                  new JSONObject()
                      .put("op", "lyrics_save")
                      .put("url", url)
                      .put("document", invalid)
                      .toString()));
      assertFalse(bad.getBoolean("ok"));
      JSONObject stored =
          PlaybackService.rpc(new JSONObject().put("op", "lyrics_get").put("url", url))
              .getJSONObject("document");
      assertEquals(document.getString("title"), stored.getString("title"));
      assertEquals(500, stored.getInt("offset_ms"));
      assertEquals(document.getString("text"), stored.getString("text"));
    } finally {
      PlaybackService.rpc(new JSONObject().put("op", "lyrics_remove").put("url", url));
    }
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
      assertEquals("result", response.getString("type"));
      JSONObject values = want.getJSONObject("challenges"), actual = response.getJSONObject("data");
      assertEquals(values.length(), actual.length());
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
