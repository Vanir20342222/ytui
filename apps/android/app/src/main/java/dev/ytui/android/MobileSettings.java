package dev.ytui.android;

import android.content.Context;
import android.content.SharedPreferences;
import java.util.*;
import org.json.*;

/** Mobile-only presentation/storage policy; desktop configuration stays portable. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class MobileSettings {
  static final Map<String, Boolean> BOOLEANS;

  static {
    Map<String, Boolean> values = new LinkedHashMap<>();
    for (String key :
        new String[] {
          "preload_next",
          "autoplay_related",
          "prefer_offline",
          "gestures",
          "background_playback",
          "show_channel",
          "show_views",
          "show_upload_date",
          "show_duration",
          "auto_save_folders"
        }) values.put(key, true);
    for (String key :
        new String[] {"preload_mobile", "show_description", "show_section", "show_likes"})
      values.put(key, false);
    BOOLEANS = Collections.unmodifiableMap(values);
  }

  static SharedPreferences prefs() {
    return MediaBridge.context.getSharedPreferences("mobile", Context.MODE_PRIVATE);
  }

  static boolean enabled(String key) {
    return prefs().getBoolean(key, BOOLEANS.getOrDefault(key, false));
  }

  static int integer(String key) {
    return prefs()
        .getInt(key, key.equals("download_parallel") ? 3 : key.equals("seek_seconds") ? 10 : 128);
  }

  static JSONObject snapshot() {
    JSONObject value = new JSONObject();
    try {
      value.put("music_folder", StorageLocations.label(true));
      value.put("video_folder", StorageLocations.label(false));
      for (String key : BOOLEANS.keySet()) value.put(key, enabled(key));
      for (String key : new String[] {"download_parallel", "seek_seconds", "stream_cache_mb"})
        value.put(key, integer(key));
    } catch (JSONException impossible) {
      throw new IllegalStateException(impossible);
    }
    return value;
  }

  static void save(JSONObject value) throws Exception {
    SharedPreferences.Editor editor = prefs().edit();
    for (String key : BOOLEANS.keySet())
      if (value.has(key)) editor.putBoolean(key, value.getBoolean(key));
    for (String key : new String[] {"download_parallel", "seek_seconds", "stream_cache_mb"})
      if (value.has(key)) {
        int number = value.getInt(key),
            low = key.equals("download_parallel") ? 1 : key.equals("seek_seconds") ? 5 : 32,
            high = key.equals("download_parallel") ? 4 : key.equals("seek_seconds") ? 30 : 512;
        if (number < low || number > high)
          throw new IllegalArgumentException("Setting outside supported range");
        editor.putInt(key, number);
      }
    if (!editor.commit()) throw new java.io.IOException("Could not save mobile settings");
    MediaBridge.main.post(
        () -> {
          if (OfflineStore.manager != null)
            OfflineStore.manager.setMaxParallelDownloads(integer("download_parallel"));
          if (PlaybackService.current != null) PlaybackService.current.refreshPrefetch();
          if (value.optBoolean("auto_save_folders")) OfflineStore.exportPending();
        });
  }

  private MobileSettings() {}
}
