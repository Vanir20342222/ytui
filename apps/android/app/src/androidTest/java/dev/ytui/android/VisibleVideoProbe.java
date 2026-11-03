package dev.ytui.android;

import android.graphics.*;
import android.os.*;
import android.view.*;
import androidx.media3.ui.PlayerView;
import androidx.test.core.app.ActivityScenario;
import java.io.*;
import java.util.concurrent.*;

/**
 * Small actual-window readback, including overlays. Request and completed evidence have distinct
 * clocks; completion is a conservative upper bound, not an exact display-presentation timestamp.
 */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class VisibleVideoProbe {
  static final class Result {
    final boolean rgb;
    final long requestedAt;
    final long capturedAt;
    String method = "window-pixelcopy";

    Result(boolean rgb, long capturedAt) {
      this(rgb, -1, capturedAt);
    }

    Result(boolean rgb, long requestedAt, long capturedAt) {
      this.rgb = rgb;
      this.requestedAt = requestedAt;
      this.capturedAt = capturedAt;
      this.method = requestedAt < 0 ? "none" : "window-pixelcopy";
    }
  }

  static final class Frame {
    final Bitmap bitmap;
    final long requestedAt;
    final long at;
    final int status;
    String method = "window-pixelcopy";

    Frame(Bitmap bitmap, long at, int status) {
      this(bitmap, -1, at, status);
    }

    Frame(Bitmap bitmap, long requestedAt, long at, int status) {
      this.bitmap = bitmap;
      this.requestedAt = requestedAt;
      this.at = at;
      this.status = status;
    }
  }

  static PlayerView find(View root) {
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
    return capture(scenario, timing, filename, 2000);
  }

  static Result capture(
      ActivityScenario<RustActivity> scenario, long timing, String filename, long budgetMs)
      throws Exception {
    return capture(action -> scenario.onActivity(action::accept), timing, filename, budgetMs);
  }

  private interface Scheduler {
    void onActivity(java.util.function.Consumer<RustActivity> action);
  }

  /** Bind before the tap: observing a frame must not wait for UI idleness. */
  static Observer observe(ActivityScenario<RustActivity> scenario) {
    java.util.concurrent.atomic.AtomicReference<RustActivity> activity =
        new java.util.concurrent.atomic.AtomicReference<>();
    scenario.onActivity(activity::set);
    return new Observer(activity.get());
  }

  static final class Observer {
    private final RustActivity activity;
    int requests;
    long observationMs;

    private Observer(RustActivity activity) {
      this.activity = activity;
    }

    Result capture(long timing, String filename, long budgetMs) throws Exception {
      requests++;
      long started = SystemClock.elapsedRealtime();
      try {
        return VisibleVideoProbe.capture(
            action -> MediaBridge.main.post(() -> action.accept(activity)),
            timing,
            filename,
            budgetMs);
      } finally {
        observationMs += SystemClock.elapsedRealtime() - started;
      }
    }
  }

  /**
   * Allow a late readback to provide diagnostics. Callers still qualify the original startup budget
   * against capturedAt (completion); requesting a copy alone does not establish its sampled pixels.
   */
  private static final long READBACK_ALLOWANCE_MS = 2000;

  private static final ExecutorService COMPOSITOR =
      new ThreadPoolExecutor(
          1,
          1,
          0,
          TimeUnit.MILLISECONDS,
          new ArrayBlockingQueue<>(1),
          runnable -> {
            Thread thread = new Thread(runnable, "compositor-evidence");
            thread.setDaemon(true);
            return thread;
          });

  /** Include the separate SurfaceView layer AND all app-window overlays. */
  private static void compositor(
      android.app.Activity activity, PlayerView view, CompletableFuture<Frame> future) {
    int[] point = new int[2];
    if (Build.VERSION.SDK_INT >= 34) view.getLocationInWindow(point);
    else view.getLocationOnScreen(point);
    Rect region =
        new Rect(point[0], point[1], point[0] + view.getWidth(), point[1] + view.getHeight());
    try {
      COMPOSITOR.execute(
          () -> {
            if (future.isDone()) return;
            Bitmap whole = null, software = null, target = null;
            long requestedAt = SystemClock.elapsedRealtime();
            try {
              android.app.UiAutomation automation =
                  androidx.test.platform.app.InstrumentationRegistry.getInstrumentation()
                      .getUiAutomation();
              whole =
                  Build.VERSION.SDK_INT >= 34
                      ? automation.takeScreenshot(activity.getWindow())
                      : automation.takeScreenshot();
              if (whole == null) throw new IllegalStateException("Compositor unavailable");
              software =
                  whole.getConfig() == Bitmap.Config.HARDWARE
                      ? whole.copy(Bitmap.Config.ARGB_8888, false)
                      : whole;
              if (software == null
                  || !region.intersect(0, 0, software.getWidth(), software.getHeight()))
                throw new IllegalStateException("Player outside compositor capture");
              target = Bitmap.createBitmap(320, 180, Bitmap.Config.ARGB_8888);
              new Canvas(target)
                  .drawBitmap(
                      software,
                      region,
                      new Rect(0, 0, 320, 180),
                      new Paint(Paint.FILTER_BITMAP_FLAG));
              Frame frame =
                  new Frame(target, requestedAt, SystemClock.elapsedRealtime(), PixelCopy.SUCCESS);
              frame.method = "window-compositor";
              if (!future.complete(frame)) target.recycle();
              target = null;
            } catch (RuntimeException unavailable) {
              if (target != null) target.recycle();
              future.complete(
                  new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
            } finally {
              if (software != null && software != whole) software.recycle();
              if (whole != null) whole.recycle();
            }
          });
    } catch (RejectedExecutionException busy) {
      future.complete(
          new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
    }
  }

  private static Result capture(Scheduler scheduler, long timing, String filename, long budgetMs)
      throws Exception {
    CompletableFuture<Frame> future = new CompletableFuture<>();
    CountDownLatch submitted = new CountDownLatch(1);
    scheduler.onActivity(
        activity -> {
          PlayerView view = find(activity.getWindow().getDecorView());
          if (activity.isDestroyed()
              || !activity.hasWindowFocus()
              || view == null
              || !view.isShown()
              || view.getWidth() < 1
              || view.getHeight() < 1) {
            future.complete(
                new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
            submitted.countDown();
            return;
          }
          if (view.getVideoSurfaceView() instanceof SurfaceView) {
            // Avoid perturbing decoder startup with repeated full compositor
            // reads. Observe a current decoded/playing source independently of
            // the later view-frame guard; deadlines still require both clocks.
            PlaybackService service = PlaybackService.current;
            if (service == null
                || service.startupToken != timing
                || view.getPlayer() != service.controlsPlayer
                || service.firstFrameMs <= 0
                || (StartupTimings.usable(timing) <= 0 && !service.player.isPlaying()))
              future.complete(
                  new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
            else compositor(activity, view, future);
            submitted.countDown();
            return;
          }
          int[] point = new int[2];
          view.getLocationInWindow(point);
          Rect region =
              new Rect(point[0], point[1], point[0] + view.getWidth(), point[1] + view.getHeight());
          Bitmap target = Bitmap.createBitmap(320, 180, Bitmap.Config.ARGB_8888);
          // PixelCopy is asynchronous and supplies no buffer-presentation timestamp.
          // Keep submission separate from the time genuine RGB evidence becomes available.
          long requestedAt = SystemClock.elapsedRealtime();
          try {
            PixelCopy.request(
                activity.getWindow(),
                region,
                target,
                status -> {
                  if (!future.complete(
                      new Frame(target, requestedAt, SystemClock.elapsedRealtime(), status)))
                    target.recycle();
                },
                MediaBridge.main);
          } catch (IllegalArgumentException unavailable) {
            target.recycle();
            future.complete(
                new Frame(null, SystemClock.elapsedRealtime(), PixelCopy.ERROR_SOURCE_NO_DATA));
          }
          submitted.countDown();
        });
    Frame frame;
    long remaining = budgetMs - StartupTimings.elapsed(timing, SystemClock.elapsedRealtime());
    try {
      if (!submitted.await(Math.max(1, remaining), TimeUnit.MILLISECONDS))
        throw new TimeoutException("Window copy was not requested within the budget");
      frame = future.get(READBACK_ALLOWANCE_MS, TimeUnit.MILLISECONDS);
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
      Result result =
          new Result(red > 100 && green > 100 && blue > 100, frame.requestedAt, frame.at);
      result.method = frame.method;
      return result;
    } finally {
      frame.bitmap.recycle();
    }
  }

  private VisibleVideoProbe() {}
}
