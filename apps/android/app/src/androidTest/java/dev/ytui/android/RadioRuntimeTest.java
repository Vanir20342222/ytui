package dev.ytui.android;

import static org.junit.Assert.*;

import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;

/** Actual HTTP/ICY demux/decoder/metadata callbacks, using generated audio only. */
@RunWith(AndroidJUnit4.class)
public final class RadioRuntimeTest {
  @Test
  public void liveIcyTitlesUpdateWithoutReplacingStationIdentity() throws Exception {
    byte[] sound;
    try (InputStream in =
            InstrumentationRegistry.getInstrumentation()
                .getContext()
                .getAssets()
                .open("fixture-radio.mp3");
        ByteArrayOutputStream out = new ByteArrayOutputStream()) {
      byte[] chunk = new byte[4096];
      int n;
      while ((n = in.read(chunk)) != -1) out.write(chunk, 0, n);
      sound = out.toByteArray();
    }
    try (ActivityScenario<RustActivity> scenario = RuntimeTest.launchOffline();
        IcyServer server = new IcyServer(sound)) {
      JSONObject media =
          new JSONObject()
              .put("url", server.url())
              .put("title", "Fixture station")
              .put("kind", "station")
              .put("live", true);
      JSONObject request =
          new JSONObject()
              .put("media", media)
              .put("audio", true)
              .put("stream", new JSONObject().put("url", server.url()).put("live", true));
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
      MediaRuntimeTest.await(
          "Live ICY metadata never reached the player",
          () -> {
            JSONObject status = MediaRuntimeTest.status();
            return status.optBoolean("playing")
                && status.optLong("stream_metadata_updates") >= 2
                && status.optString("title").equals("Fixture artist - Second song");
          });
      JSONObject status = MediaRuntimeTest.status();
      assertEquals("Fixture station", status.getJSONObject("media").getString("title"));
      assertEquals("Fixture station", status.getString("station"));
      assertEquals("Fixture artist - Second song", status.getString("stream_title"));
      java.util.concurrent.atomic.AtomicReference<String> sessionTitle =
          new java.util.concurrent.atomic.AtomicReference<>("");
      MediaBridge.main.post(
          () ->
              sessionTitle.set(
                  String.valueOf(
                      PlaybackService.current.session.getPlayer().getMediaMetadata().title)));
      MediaRuntimeTest.await(
          "MediaSession did not receive the live song title",
          () -> "Fixture artist - Second song".equals(sessionTitle.get()));
      JSONArray tracks = status.getJSONArray("tracks");
      for (int i = 0; i < tracks.length(); i++)
        assertFalse(
            "A metadata track was mislabeled as video",
            tracks.getJSONObject(i).getString("label").startsWith("Video"));
      MediaRuntimeTest.command("pause");
      MediaRuntimeTest.await(
          "Live pause was not applied", () -> MediaRuntimeTest.status().optBoolean("paused"));
    } finally {
      MediaRuntimeTest.command("stop");
    }
  }

  static final class IcyServer implements AutoCloseable {
    final ServerSocket listener = new ServerSocket(0, 4, InetAddress.getByName("127.0.0.1"));
    final Thread thread;

    IcyServer(byte[] sound) throws Exception {
      ByteArrayOutputStream body = new ByteArrayOutputStream();
      int interval = 8192;
      for (int offset = 0, index = 0; offset < sound.length; index++) {
        int count = Math.min(interval, sound.length - offset);
        body.write(sound, offset, count);
        offset += count;
        if (count == interval) {
          byte[] metadata =
              ("StreamTitle='Fixture artist - "
                      + (index == 0 ? "First song" : "Second song")
                      + "';")
                  .getBytes(StandardCharsets.ISO_8859_1);
          int blocks = (metadata.length + 15) / 16;
          body.write(blocks);
          body.write(metadata);
          body.write(new byte[blocks * 16 - metadata.length]);
        }
      }
      byte[] payload = body.toByteArray();
      thread =
          new Thread(
              () -> {
                while (!listener.isClosed()) {
                  try (Socket client = listener.accept()) {
                    client.setSoTimeout(5000);
                    BufferedReader reader =
                        new BufferedReader(
                            new InputStreamReader(
                                client.getInputStream(), StandardCharsets.US_ASCII));
                    String line;
                    while ((line = reader.readLine()) != null && !line.isEmpty()) {}
                    OutputStream out = client.getOutputStream();
                    out.write(
                        ("HTTP/1.1 200 OK\r\nContent-Type: audio/mpeg\r\nicy-metaint: "
                                + interval
                                + "\r\nicy-name: Fixture station\r\nContent-Length: "
                                + payload.length
                                + "\r\nConnection: close\r\n\r\n")
                            .getBytes(StandardCharsets.US_ASCII));
                    out.write(payload);
                    out.flush();
                  } catch (IOException ignored) {
                  }
                }
              },
              "local-icy-fixture");
      thread.start();
    }

    String url() {
      return "http://127.0.0.1:" + listener.getLocalPort() + "/radio.mp3";
    }

    @Override
    public void close() throws IOException {
      listener.close();
      try {
        thread.join(6000);
      } catch (InterruptedException e) {
        Thread.currentThread().interrupt();
      }
    }
  }
}
