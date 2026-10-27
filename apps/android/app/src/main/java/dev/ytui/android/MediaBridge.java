package dev.ytui.android;

import android.content.*;
import android.content.res.Configuration;
import android.graphics.SurfaceTexture;
import android.os.*;
import android.view.Surface;
import java.util.concurrent.ConcurrentLinkedQueue;
import org.json.*;

/** Platform adapters only. UI, themes, providers and application state are Rust. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class MediaBridge {
  static android.app.Application context;
  static final Handler main = new Handler(Looper.getMainLooper());
  static final ConcurrentLinkedQueue<JSONObject> events = new ConcurrentLinkedQueue<>();
  static volatile String playerSnapshot = "{}";
  static SurfaceTexture texture;
  static Surface surface;
  static int textureId;
  static volatile int topInset, bottomInset;
  static final java.util.concurrent.atomic.AtomicLong videoFrames =
      new java.util.concurrent.atomic.AtomicLong();
  static final java.util.concurrent.atomic.AtomicBoolean frameReady =
      new java.util.concurrent.atomic.AtomicBoolean();

  static void attach(Context ctx) {
    context = (android.app.Application) ctx.getApplicationContext();
    try {
      NetworkSettings.apply(
          new JSONObject(NativeBridge.request("{\"op\":\"settings\"}")).getJSONObject("data"));
    } catch (Exception ignored) {
    }
  }

  static void event(String kind, String text) {
    try {
      events.add(new JSONObject().put("kind", kind).put("text", text == null ? "" : text));
    } catch (JSONException ignored) {
    }
  }

  public static String evaluate(String script) {
    return ScriptRuntime.evaluate(script);
  }

  public static String snapshot() {
    try {
      JSONObject value =
          new JSONObject().put("inset_top", topInset).put("inset_bottom", bottomInset);
      value.put(
          "dark",
          (context.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
              == Configuration.UI_MODE_NIGHT_YES);
      value.put("player", new JSONObject(playerSnapshot));
      JSONArray downloads = OfflineStore.changedSnapshot();
      if (downloads != null) value.put("downloads", downloads);
      JSONArray pending = new JSONArray();
      for (int i = 0; i < 32; i++) {
        JSONObject event = events.poll();
        if (event == null) break;
        pending.put(event);
      }
      value.put("events", pending);
      return value.toString();
    } catch (Exception error) {
      return "{}";
    }
  }

  public static void execute(String raw) {
    main.post(
        () -> {
          try {
            JSONObject request = new JSONObject(raw);
            String op = request.optString("op");
            switch (op) {
              case "background":
                if (RustActivity.current() != null) RustActivity.current().moveTaskToBack(true);
                break;
              case "play":
                {
                  Intent intent =
                      new Intent(context, PlaybackService.class)
                          .setAction("play")
                          .putExtra("request", request.getJSONObject("request").toString());
                  context.startForegroundService(intent);
                  break;
                }
              case "download_playlist":
                PrepareWorker.enqueue(request);
                break;
              case "download":
                OfflineStore.add(request.getJSONObject("request"));
                break;
              case "offline":
                {
                  JSONObject item = OfflineStore.offlineRequest(request.getString("id"));
                  Intent intent =
                      new Intent(context, PlaybackService.class)
                          .setAction("play")
                          .putExtra("request", item.toString());
                  context.startForegroundService(intent);
                  break;
                }
              case "download_remove":
                OfflineStore.remove(request.getString("id"));
                break;
              case "download_toggle":
                OfflineStore.toggle(request.getString("id"));
                break;
              case "open":
                context.startActivity(
                    new Intent(Intent.ACTION_VIEW, android.net.Uri.parse(request.getString("url")))
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                break;
              case "pick":
                {
                  RustActivity activity = RustActivity.current();
                  if (activity != null)
                    activity.startActivityForResult(
                        new Intent(Intent.ACTION_OPEN_DOCUMENT)
                            .setType("*/*")
                            .addCategory(Intent.CATEGORY_OPENABLE),
                        "theme".equals(request.optString("kind")) ? 42 : 41);
                  break;
                }
              case "login":
                if (NetworkSettings.explicitProxy) {
                  event("error", "Use cookie-file import when an explicit proxy is configured");
                  break;
                }
                context.startActivity(
                    new Intent(context, AuthActivity.class)
                        .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK));
                break;
              case "disconnect":
                android.webkit.CookieManager.getInstance().removeAllCookies(null);
                if (PlaybackService.current != null) PlaybackService.current.stopAndClear();
                break;
              case "settings":
                NetworkSettings.apply(request.getJSONObject("config"));
                if (PlaybackService.current != null) PlaybackService.current.command(request);
                break;
              default:
                if (PlaybackService.current != null) PlaybackService.current.command(request);
                break;
            }
          } catch (Exception error) {
            event("error", "Android operation failed: " + error.getClass().getSimpleName());
          }
        });
  }

  public static synchronized float[] videoFrame(int id) {
    try {
      if (texture == null || textureId != id) {
        if (surface != null) surface.release();
        if (texture != null) texture.release();
        textureId = id;
        texture = new SurfaceTexture(id);
        texture.setDefaultBufferSize(1920, 1080);
        surface = new Surface(texture);
        texture.setOnFrameAvailableListener(
            t -> {
              videoFrames.incrementAndGet();
              frameReady.set(true);
              NativeBridge.frameAvailable();
            });
        Surface ready = surface;
        main.post(
            () -> {
              if (PlaybackService.current != null)
                PlaybackService.current.player.setVideoSurface(ready);
            });
      }
      if (frameReady.getAndSet(false)) texture.updateTexImage();
      float[] matrix = new float[16];
      texture.getTransformMatrix(matrix);
      return matrix;
    } catch (RuntimeException error) {
      return null;
    }
  }

  public static void releaseVideo(String unused) {
    main.post(
        () -> {
          if (PlaybackService.current != null) PlaybackService.current.player.clearVideoSurface();
          synchronized (MediaBridge.class) {
            if (surface != null) surface.release();
            if (texture != null) texture.release();
            surface = null;
            texture = null;
            textureId = 0;
          }
        });
  }

  private MediaBridge() {}
}
