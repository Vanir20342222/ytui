package dev.ytui.android;

import android.view.GestureDetector;
import android.view.MotionEvent;
import androidx.media3.ui.PlayerView;
import org.json.JSONObject;

/** Double-tap on the video surface; native controls keep their own touch handling. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class VideoGestures {
  static void attach(PlayerView view) {
    GestureDetector detector =
        new GestureDetector(
            view.getContext(),
            new GestureDetector.SimpleOnGestureListener() {
              @Override
              public boolean onDoubleTap(MotionEvent event) {
                PlaybackService service = PlaybackService.current;
                if (service == null || !MobileSettings.enabled("gestures")) return false;
                try {
                  service.command(
                      new JSONObject()
                          .put("op", event.getX() < view.getWidth() / 2f ? "back" : "forward"));
                } catch (Exception ignored) {
                }
                return true;
              }
            });
    view.setOnTouchListener((target, event) -> detector.onTouchEvent(event));
  }

  private VideoGestures() {}
}
