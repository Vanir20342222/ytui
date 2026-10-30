package dev.ytui.android;

import androidx.media3.exoplayer.offline.DownloadManager;
import androidx.media3.exoplayer.scheduler.Requirements;
import androidx.test.platform.app.InstrumentationRegistry;

/** Local HTTP fixtures do not require Android's public-Internet validation probe. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
final class LoopbackDownloads implements AutoCloseable {
  private DownloadManager manager;
  private Requirements original;
  private final DownloadManager.Listener listener =
      new DownloadManager.Listener() {
        @Override
        public void onDownloadChanged(
            DownloadManager manager,
            androidx.media3.exoplayer.offline.Download download,
            Exception error) {
          android.util.Log.i(
              "LoopbackDownloads",
              "state="
                  + download.state
                  + " bytes="
                  + download.getBytesDownloaded()
                  + " failure="
                  + download.failureReason
                  + " cause="
                  + (error == null ? "none" : error.getClass().getSimpleName()));
        }
      };

  static LoopbackDownloads open() {
    if (!BuildConfig.DEBUG
        || !MediaBridge.offlineUi
        || !MediaBridge.context.getPackageName().endsWith(".preview"))
      throw new IllegalStateException("Requires the disposable offline test installation");
    LoopbackDownloads scope = new LoopbackDownloads();
    InstrumentationRegistry.getInstrumentation()
        .runOnMainSync(
            () -> {
              scope.manager = OfflineStore.manager();
              scope.original = scope.manager.getRequirements();
              scope.manager.addListener(scope.listener);
              android.util.Log.i(
                  "LoopbackDownloads",
                  "requirements="
                      + scope.original.getRequirements()
                      + " unmet="
                      + scope.manager.getNotMetRequirements());
              scope.manager.setRequirements(new Requirements(0));
            });
    return scope;
  }

  @Override
  public void close() {
    InstrumentationRegistry.getInstrumentation()
        .runOnMainSync(
            () -> {
              manager.removeListener(listener);
              manager.setRequirements(original);
              org.junit.Assert.assertEquals(
                  original.getRequirements(), manager.getRequirements().getRequirements());
            });
  }
}
