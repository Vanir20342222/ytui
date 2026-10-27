package dev.ytui.android;

import static org.junit.Assert.*;

import androidx.test.core.app.ActivityScenario;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import java.io.*;
import java.net.*;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;
import org.json.*;
import org.junit.*;
import org.junit.runner.RunWith;

/** Real Media3 EOF/history transitions against an isolated HTTP collection and media server. */
@RunWith(AndroidJUnit4.class)
public final class PlaybackNavigationTest {
  @Rule
  public androidx.test.rule.GrantPermissionRule notifications =
      androidx.test.rule.GrantPermissionRule.grant(
          android.os.Build.VERSION.SDK_INT >= 33
              ? new String[] {"android.permission.POST_NOTIFICATIONS"}
              : new String[] {});

  static JSONObject media(CollectionServer server, String id) throws Exception {
    return new JSONObject()
        .put("id", id)
        .put("title", id)
        .put("url", server.url("/" + id + ".m4a"))
        .put("kind", "song");
  }

  static void playing(String title) throws Exception {
    MediaRuntimeTest.await(
        "Did not play " + title,
        () ->
            MediaRuntimeTest.status().optString("title").equals(title)
                && MediaRuntimeTest.status().optBoolean("playing")
                && MediaRuntimeTest.status().optDouble("position") > 0.1);
  }

  static void clearQueue() throws Exception {
    JSONArray queue = PlaybackService.rpcArray(new JSONObject().put("op", "queue"));
    for (int i = 0; i < queue.length(); i++)
      PlaybackService.rpc(new JSONObject().put("op", "queue_remove").put("index", 0));
  }

  static void remote(androidx.media3.session.MediaController controller, boolean previous)
      throws Exception {
    CompletableFuture<Void> completed = new CompletableFuture<>();
    MediaBridge.main.post(
        () -> {
          try {
            int command =
                previous
                    ? androidx.media3.common.Player.COMMAND_SEEK_TO_PREVIOUS_MEDIA_ITEM
                    : androidx.media3.common.Player.COMMAND_SEEK_TO_NEXT_MEDIA_ITEM;
            assertTrue(
                "MediaSession did not expose queue navigation",
                controller.isCommandAvailable(command));
            if (previous) controller.seekToPreviousMediaItem();
            else controller.seekToNextMediaItem();
            completed.complete(null);
          } catch (Throwable error) {
            completed.completeExceptionally(error);
          }
        });
    completed.get(5, TimeUnit.SECONDS);
  }

  @Test
  public void collectionEofManualQueuePreviousForwardAndFailedNextKeepTheirOrder()
      throws Exception {
    JSONObject original = PlaybackService.rpc(new JSONObject().put("op", "settings"));
    byte[] audio;
    try (InputStream input =
        InstrumentationRegistry.getInstrumentation()
            .getContext()
            .getAssets()
            .open("fixture-audio.m4a")) {
      ByteArrayOutputStream bytes = new ByteArrayOutputStream();
      byte[] buffer = new byte[8192];
      int count;
      while ((count = input.read(buffer)) != -1) bytes.write(buffer, 0, count);
      audio = bytes.toByteArray();
    }
    try (ActivityScenario<RustActivity> activity = RuntimeTest.launchOffline();
        CollectionServer server = new CollectionServer(audio)) {
      assertEquals("offline", MediaBridge.uiMode());
      MediaBridge.playbackStore
          .submit(
              () -> {
                PlaybackService.rpcArray(new JSONObject().put("op", "history_clear"));
                return null;
              })
          .get(5, TimeUnit.SECONDS);
      clearQueue();
      JSONObject config =
          new JSONObject(original.toString())
              .put("proxy", server.url(""))
              .put("playback_history", true);
      PlaybackService.rpc(new JSONObject().put("op", "save_settings").put("config", config));
      String token = "local-playback-fixture";
      JSONObject descriptor =
          new JSONObject()
              .put("op", "prepare_context")
              .put("context_id", token)
              .put("source_id", "First")
              .put("source_url", server.url("/First.m4a"))
              .put(
                  "request",
                  new JSONObject().put("CliampTracks", "http://radio.cliamp.stream/library"));
      JSONObject request =
          new JSONObject()
              .put("media", media(server, "First"))
              .put("stream", new JSONObject().put("url", server.url("/First.m4a")))
              .put("audio", true)
              .put("context_id", token)
              .put("context", descriptor);
      MediaBridge.execute(new JSONObject().put("op", "play").put("request", request).toString());
      playing("First");
      // No Next action: the actual decoder's EOF must advance to the collection tail.
      playing("Second");
      MediaRuntimeTest.await(
          "Second song was not recorded",
          () -> PlaybackService.rpcArray(new JSONObject().put("op", "history_list")).length() == 2);
      androidx.media3.session.SessionToken sessionToken =
          new androidx.media3.session.SessionToken(
              InstrumentationRegistry.getInstrumentation().getTargetContext(),
              new android.content.ComponentName(
                  InstrumentationRegistry.getInstrumentation().getTargetContext(),
                  PlaybackService.class));
      androidx.media3.session.MediaController controller =
          new androidx.media3.session.MediaController.Builder(
                  InstrumentationRegistry.getInstrumentation().getTargetContext(), sessionToken)
              .setApplicationLooper(android.os.Looper.getMainLooper())
              .buildAsync()
              .get(10, TimeUnit.SECONDS);
      try {
        remote(controller, true);
        playing("First");
        MediaRuntimeTest.await(
            "Previous history position was not committed",
            () ->
                PlaybackService.rpc(
                        new JSONObject().put("op", "history_peek").put("direction", "next"))
                    .has("media"));
        remote(controller, false);
        playing("Second");
        PlaybackService.rpc(
            new JSONObject().put("op", "enqueue").put("media", media(server, "Manual")));
        remote(controller, false);
        playing("Manual");
        remote(controller, false);
        playing("Third");
      } finally {
        MediaBridge.main.post(controller::release);
      }
      assertFalse(MediaRuntimeTest.status().optBoolean("video"));
      PlaybackService.rpc(
          new JSONObject().put("op", "enqueue").put("media", media(server, "Missing")));
      MediaRuntimeTest.command("next");
      MediaRuntimeTest.await(
          "Unavailable next track did not fail", () -> !MediaRuntimeTest.status().isNull("error"));
      JSONArray queue = PlaybackService.rpcArray(new JSONObject().put("op", "queue"));
      assertEquals("Missing", queue.getJSONObject(0).getString("title"));
      JSONArray history = PlaybackService.rpcArray(new JSONObject().put("op", "history_list"));
      for (int i = 0; i < history.length(); i++)
        assertNotEquals("Missing", history.getJSONObject(i).getString("title"));
    } finally {
      MediaRuntimeTest.command("stop");
      MediaBridge.playbackStore.submit(() -> null).get(5, TimeUnit.SECONDS);
      clearQueue();
      PlaybackService.rpc(new JSONObject().put("op", "save_settings").put("config", original));
    }
  }

  static final class CollectionServer implements AutoCloseable {
    final ServerSocket socket = new ServerSocket(0, 10, InetAddress.getByName("127.0.0.1"));
    final ExecutorService clients = Executors.newCachedThreadPool();
    final Thread acceptor;
    final byte[] audio;

    CollectionServer(byte[] audio) throws Exception {
      this.audio = audio;
      acceptor =
          new Thread(
              () -> {
                while (!socket.isClosed()) {
                  try {
                    Socket client = socket.accept();
                    clients.execute(() -> serve(client));
                  } catch (Exception error) {
                    if (!socket.isClosed()) throw new RuntimeException(error);
                  }
                }
              },
              "collection-fixture");
      acceptor.start();
    }

    String url(String path) {
      return "http://127.0.0.1:" + socket.getLocalPort() + path;
    }

    void serve(Socket client) {
      try (Socket connection = client) {
        connection.setSoTimeout(10000);
        BufferedReader input =
            new BufferedReader(
                new InputStreamReader(connection.getInputStream(), StandardCharsets.US_ASCII));
        String first = input.readLine();
        if (first == null) return;
        boolean head = first.startsWith("HEAD");
        int offset = 0;
        String line;
        while ((line = input.readLine()) != null && !line.isEmpty())
          if (line.toLowerCase(java.util.Locale.ROOT).startsWith("range: bytes="))
            offset = Integer.parseInt(line.substring(13).split("-")[0]);
        byte[] body = audio;
        String mime = "audio/mp4";
        boolean library = first.contains("/library");
        boolean missing = first.contains("/Missing.m4a");
        if (library) {
          JSONArray tracks = new JSONArray();
          for (String id : new String[] {"First", "Second", "Third"}) tracks.put(media(this, id));
          body =
              new JSONObject()
                  .put("name", "Local fixture")
                  .put("tracks", tracks)
                  .toString()
                  .getBytes(StandardCharsets.UTF_8);
          mime = "application/json";
        }
        if (missing) body = new byte[0];
        offset = Math.max(0, Math.min(offset, body.length));
        String headers =
            "HTTP/1.1 "
                + (missing ? "404 Not Found" : offset > 0 ? "206 Partial Content" : "200 OK")
                + "\r\nContent-Type: "
                + mime
                + "\r\nContent-Length: "
                + (body.length - offset)
                + "\r\nAccept-Ranges: bytes\r\n"
                + (offset > 0
                    ? "Content-Range: bytes "
                        + offset
                        + "-"
                        + (body.length - 1)
                        + "/"
                        + body.length
                        + "\r\n"
                    : "")
                + "Connection: close\r\n\r\n";
        OutputStream output = connection.getOutputStream();
        output.write(headers.getBytes(StandardCharsets.US_ASCII));
        if (!head) output.write(body, offset, body.length - offset);
        output.flush();
      } catch (Exception ignored) {
      }
    }

    @Override
    public void close() throws Exception {
      socket.close();
      acceptor.join(2000);
      clients.shutdownNow();
      clients.awaitTermination(2, TimeUnit.SECONDS);
    }
  }
}
