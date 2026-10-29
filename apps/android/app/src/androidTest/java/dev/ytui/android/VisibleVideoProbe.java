package dev.ytui.android;

import android.graphics.*;
import android.os.*;
import android.view.*;
import androidx.media3.ui.PlayerView;
import androidx.test.core.app.ActivityScenario;
import java.io.*;
import java.util.concurrent.*;

/**
 * Small GPU window readback, including overlays; no full-display screenshot latency in the budget.
 */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class VisibleVideoProbe {
  static final class Result {
    final boolean rgb;
    final long capturedAt;

    Result(boolean rgb, long capturedAt) {
      this.rgb = rgb;
      this.capturedAt = capturedAt;
    }
  }

  static final class Frame {
    final Bitmap bitmap;
    final long at;
    final int status;

    Frame(Bitmap bitmap, long at, int status) {
      this.bitmap = bitmap;
      this.at = at;
      this.status = status;
    }
  }

  private static PlayerView find(View root) {
    if (root instanceof PlayerView) return (PlayerView) root;
    if (root instanceof ViewGroup) {
      ViewGroup group = (ViewGroup) root;
      for (int i = 0; i < group.getChildCount(); i++) {
        PlayerView found = find(group.getChildAt(i));
        if (found != null) return found;
      }
    }
    return null;
  }

  static Result capture(ActivityScenario<RustActivity> scenario, long timing, String filename)
      throws Exception {
    CompletableFuture<Frame> future = new CompletableFuture<>();
    scenario.onActivity(
        activity -> {
          PlayerView view = find(activity.getWindow().getDecorView());
          if (!activity.hasWindowFocus()
              || view == null
              || !view.isShown()
              || view.getWidth() < 1
              || view.getHeight() < 1) {
            future.complete(
                new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
            return;
          }
          int[] point = new int[2];
          view.getLocationInWindow(point);
          Rect region =
              new Rect(point[0], point[1], point[0] + view.getWidth(), point[1] + view.getHeight());
          Bitmap target = Bitmap.createBitmap(320, 180, Bitmap.Config.ARGB_8888);
          try {
            PixelCopy.request(
                activity.getWindow(),
                region,
                target,
                status -> {
                  if (!future.complete(new Frame(target, SystemClock.elapsedRealtime(), status)))
                    target.recycle();
                },
                MediaBridge.main);
          } catch (IllegalArgumentException unavailable) {
            target.recycle();
            future.complete(
                new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
          }
        });
    Frame frame;
    long remaining = 2000 - StartupTimings.elapsed(timing, SystemClock.elapsedRealtime());
    try {
      frame = future.get(Math.max(1, remaining), TimeUnit.MILLISECONDS);
    } catch (TimeoutException late) {
      if (!future.cancel(false)) {
        Frame completed = future.getNow(null);
        if (completed != null && completed.bitmap != null) completed.bitmap.recycle();
      }
      return new Result(false, SystemClock.elapsedRealtime());
    }
    if (frame.bitmap == null) return new Result(false, frame.at);
    try {
      int red = 0, green = 0, blue = 0;
      if (frame.status == PixelCopy.SUCCESS) {
        for (int y = 0; y < frame.bitmap.getHeight(); y += 2)
          for (int x = 0; x < frame.bitmap.getWidth(); x += 2) {
            int pixel = frame.bitmap.getPixel(x, y);
            int r = Color.red(pixel), g = Color.green(pixel), b = Color.blue(pixel);
            if (r > 70 && g < 45 && b < 45) red++;
            if (g > 70 && r < 45 && b < 45) green++;
            if (b > 70 && r < 45 && g < 45) blue++;
          }
      }
      try (FileOutputStream out =
          new FileOutputStream(new File(MediaBridge.context.getExternalFilesDir(null), filename))) {
        frame.bitmap.compress(Bitmap.CompressFormat.PNG, 100, out);
      }
      return new Result(red > 100 && green > 100 && blue > 100, frame.at);
    } finally {
      frame.bitmap.recycle();
    }
  }

  private VisibleVideoProbe() {}
}
