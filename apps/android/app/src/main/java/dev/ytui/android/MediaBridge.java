package dev.ytui.android;

import android.content.*;
import android.content.res.Configuration;
import android.graphics.SurfaceTexture;
import android.os.*;
import android.view.Surface;
import java.util.concurrent.ConcurrentLinkedQueue;
import org.json.*;

/** Android platform adapters; providers, theme palettes and durable application state are Rust. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class MediaBridge {
  static android.app.Application context;
  static final Handler main = new Handler(Looper.getMainLooper());
  static final ConcurrentLinkedQueue<JSONObject> events = new ConcurrentLinkedQueue<>();
  // Keep accepted starts ordered across Activity and service recreation.
  static final java.util.concurrent.ExecutorService playbackStore =
      java.util.concurrent.Executors.newSingleThreadExecutor();
  static volatile String playerSnapshot = "{}";
  static SurfaceTexture texture;
  static Surface surface;
  static int textureId;
  static volatile int videoWidth = 1280, videoHeight = 720;
  static volatile int topInset, bottomInset, leftInset, rightInset;
  static volatile boolean uiVisible;
  static volatile boolean offlineUi;

  public static String uiMode() {
    return offlineUi ? "offline" : "network";
  }

  static void notifyUi() {
    if (uiVisible) ComposeHost.changed();
  }

  static final java.util.concurrent.atomic.AtomicLong videoFrames =
      new java.util.concurrent.atomic.AtomicLong();
  static final java.util.concurrent.atomic.AtomicBoolean frameReady =
      new java.util.concurrent.atomic.AtomicBoolean();

  static void attach(Context ctx) {
    context = (android.app.Application) ctx.getApplicationContext();
    main.postDelayed(ScriptRuntime::warm, 800);
    main.post(
        () -> {
          try {
            OfflineStore.manager();
          } catch (Exception error) {
            event("error", "Offline library could not be restored");
          }
        });
    try {
      NetworkSettings.apply(
          new JSONObject(NativeBridge.request("{\"op\":\"settings\"}")).getJSONObject("data"));
    } catch (Exception ignored) {
    }
  }

  static void event(String kind, String text) {
    event(kind, text, null, 0);
  }

  static void event(String kind, String text, String token, long accountIntent) {
    // Success/progress feedback belongs in controls and library state, never an overlay.
    if ("notice".equals(kind)) return;
    try {
      events.add(
          new JSONObject()
              .put("kind", kind)
              .put("text", text == null ? "" : text)
              .put("request_token", token)
              .put("account_intent", accountIntent));
      ComposeHost.changed();
    } catch (JSONException ignored) {
    }
  }

  public static String evaluate(String script) {
    return ScriptRuntime.evaluate(script);
  }

  public static String sealAccount(String text) throws Exception {
    return AccountCipher.seal(text);
  }

  public static String openAccount(String envelope) throws Exception {
    return AccountCipher.open(envelope);
  }

  public static String snapshot() {
    try {
      JSONObject value =
          new JSONObject()
              .put("app_version", BuildConfig.VERSION_NAME + " (" + BuildConfig.VERSION_CODE + ")")
              .put("inset_top", topInset)
              .put("inset_bottom", bottomInset)
              .put("inset_left", leftInset)
              .put("inset_right", rightInset);
      value.put(
          "dark",
          (context.getResources().getConfiguration().uiMode & Configuration.UI_MODE_NIGHT_MASK)
              == Configuration.UI_MODE_NIGHT_YES);
      JSONObject player = new JSONObject(playerSnapshot);
      if (AudioTap.enabled) {
        long packed = AudioTap.levels;
        player
            .put("left", (packed & 65535) / 65535.0)
            .put("right", ((packed >>> 16) & 65535) / 65535.0)
            .put("peak", ((packed >>> 32) & 65535) / 65535.0);
      }
      value.put("player", player);
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
              case "theme":
                {
                  RustActivity activity = RustActivity.current();
                  if (activity != null) {
                    boolean light = request.optBoolean("light");
                    org.json.JSONArray rgb = request.getJSONArray("background");
                    int color =
                        android.graphics.Color.rgb(rgb.getInt(0), rgb.getInt(1), rgb.getInt(2));
                    activity.getWindow().setStatusBarColor(color);
                    activity.getWindow().setNavigationBarColor(color);
                    if (android.os.Build.VERSION.SDK_INT >= 30) {
                      android.view.WindowInsetsController controller =
                          activity.getWindow().getInsetsController();
                      if (controller != null) {
                        int flags =
                            android.view.WindowInsetsController.APPEARANCE_LIGHT_STATUS_BARS
                                | android.view.WindowInsetsController
                                    .APPEARANCE_LIGHT_NAVIGATION_BARS;
                        controller.setSystemBarsAppearance(light ? flags : 0, flags);
                      }
                    } else {
                      android.view.View decor = activity.getWindow().getDecorView();
                      int flags =
                          android.view.View.SYSTEM_UI_FLAG_LIGHT_STATUS_BAR
                              | android.view.View.SYSTEM_UI_FLAG_LIGHT_NAVIGATION_BAR;
                      decor.setSystemUiVisibility(
                          light
                              ? decor.getSystemUiVisibility() | flags
                              : decor.getSystemUiVisibility() & ~flags);
                    }
                  }
                  break;
                }
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
                OfflineStore.IO.execute(
                    () -> {
                      try {
                        JSONObject item = OfflineStore.offlineRequest(request.getString("id"));
                        if (request.has("context"))
                          item.put("context", request.getJSONObject("context"));
                        if (request.has("context_id"))
                          item.put("context_id", request.getString("context_id"));
                        main.post(
                            () ->
                                context.startForegroundService(
                                    new Intent(context, PlaybackService.class)
                                        .setAction("play")
                                        .putExtra("request", item.toString())));
                      } catch (Exception error) {
                        event(
                            "error",
                            "Offline media is unavailable; reconnect its folder or save it again");
                      }
                    });
                break;
              case "folder_pick":
                if (RustActivity.current() != null)
                  RustActivity.current().pickFolder(request.optBoolean("audio"));
                break;
              case "folder_retry":
                OfflineStore.exportPending();
                break;
              case "library_visible":
                OfflineStore.libraryVisible = request.optBoolean("value");
                if (OfflineStore.libraryVisible) OfflineStore.dirty = true;
                break;
              case "download_remove":
                OfflineStore.remove(request.getString("id"));
                break;
              case "download_toggle":
                OfflineStore.toggle(request.getString("id"));
                break;
              case "download_export":
                if (RustActivity.current() != null)
                  RustActivity.current().exportDownload(request.getString("id"));
                break;
              case "history_clear":
                playbackStore.execute(
                    () -> {
                      try {
                        PlaybackService.rpcArray(new JSONObject().put("op", "history_clear"));
                        event("history_cleared", "");
                      } catch (Exception error) {
                        event("error", "Could not clear playback history");
                      }
                    });
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
                    activity.pickDocument(
                        request.optString("kind"), request.optString("request_token"));
                  break;
                }
              case "login":
                if (NetworkSettings.explicitProxy) {
                  event("error", "Use cookie-file import when an explicit proxy is configured");
                  break;
                }
                context.startActivity(
                    new Intent(context, AuthActivity.class)
                        .putExtra("ytui.offline-auth", BuildConfig.DEBUG && offlineUi)
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
        texture.setDefaultBufferSize(videoWidth, videoHeight);
        surface = new Surface(texture);
        texture.setOnFrameAvailableListener(
            t -> {
              videoFrames.incrementAndGet();
              frameReady.set(true);
              notifyUi();
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
