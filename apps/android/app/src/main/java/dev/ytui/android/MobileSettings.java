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
          "resume_on_play",
          "prefer_offline",
          "gestures",
          "background_playback",
          "show_channel",
          "show_views",
          "show_upload_date",
          "show_duration",
          "auto_save_folders",
          "animate_cards",
          "haptics",
          "gesture_swipe_tabs",
          "gesture_swipe_dismiss",
          "gesture_mini_player",
          "gesture_swipe_tracks",
          "gesture_double_tap_seek",
          "gesture_hold_speed",
          "gesture_video_adjust",
          "gesture_pinch_zoom"
        }) values.put(key, true);
    for (String key :
        new String[] {"preload_mobile", "show_description", "show_section", "show_likes"})
      values.put(key, false);
    BOOLEANS = Collections.unmodifiableMap(values);
    Map<String, List<String>> choices = new LinkedHashMap<>();
    // First entry is the default.
    choices.put("animation_style", Collections.unmodifiableList(Arrays.asList("smooth", "snappy", "bouncy", "gentle")));
    STRINGS = Collections.unmodifiableMap(choices);
  }

  /** Enumerated text settings; the first value is the default. */
  static final Map<String, List<String>> STRINGS;

  private static final String[] INTEGERS = {
    "download_parallel", "seek_seconds", "stream_cache_mb", "animation_speed", "hold_speed"
  };

  private static int fallback(String key) {
    switch (key) {
      case "download_parallel":
        return 3;
      case "seek_seconds":
        return 10;
      case "animation_speed":
        return 100;
      case "hold_speed":
        return 200;
      default:
        return 128;
    }
  }

  private static int[] range(String key) {
    switch (key) {
      case "download_parallel":
        return new int[] {1, 4};
      case "seek_seconds":
        return new int[] {5, 30};
      case "animation_speed":
        return new int[] {50, 200}; // percent; higher is faster
      case "hold_speed":
        return new int[] {125, 300}; // percent playback rate while held
      default:
        return new int[] {32, 512};
    }
  }

  static String text(String key) {
    List<String> allowed = STRINGS.get(key);
    if (allowed == null) return "";
    String value = prefs().getString(key, allowed.get(0));
    return allowed.contains(value) ? value : allowed.get(0);
  }

  /** Master gesture switch plus the individual gesture. */
  static boolean gesture(String key) {
    return enabled("gestures") && enabled(key);
  }

  static SharedPreferences prefs() {
    return MediaBridge.context.getSharedPreferences("mobile", Context.MODE_PRIVATE);
  }

  static boolean enabled(String key) {
    return prefs().getBoolean(key, BOOLEANS.getOrDefault(key, false));
  }

  static int integer(String key) {
    return prefs().getInt(key, fallback(key));
  }

  static JSONObject snapshot() {
    JSONObject value = new JSONObject();
    try {
      value.put("music_folder", StorageLocations.label(true));
      value.put("video_folder", StorageLocations.label(false));
      for (String key : BOOLEANS.keySet()) value.put(key, enabled(key));
      for (String key : INTEGERS) value.put(key, integer(key));
      for (String key : STRINGS.keySet()) value.put(key, text(key));
    } catch (JSONException impossible) {
      throw new IllegalStateException(impossible);
    }
    return value;
  }

  static void save(JSONObject value) throws Exception {
    SharedPreferences.Editor editor = prefs().edit();
    for (String key : BOOLEANS.keySet())
      if (value.has(key)) editor.putBoolean(key, value.getBoolean(key));
    for (String key : INTEGERS)
      if (value.has(key)) {
        int number = value.getInt(key);
        int[] bounds = range(key);
        if (number < bounds[0] || number > bounds[1])
          throw new IllegalArgumentException("Setting outside supported range");
        editor.putInt(key, number);
      }
    for (Map.Entry<String, List<String>> entry : STRINGS.entrySet())
      if (value.has(entry.getKey())) {
        String text = value.getString(entry.getKey());
        if (!entry.getValue().contains(text))
          throw new IllegalArgumentException("Unsupported setting value");
        editor.putString(entry.getKey(), text);
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
