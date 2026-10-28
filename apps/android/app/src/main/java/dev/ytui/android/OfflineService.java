package dev.ytui.android;

import android.app.*;
import androidx.media3.exoplayer.offline.*;
import androidx.media3.exoplayer.scheduler.Scheduler;
import java.util.List;

@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class OfflineService extends DownloadService {
  public OfflineService() {
    super(NotificationIds.DOWNLOADS, 1000, "ytui-downloads", R.string.downloads_channel_name, 0);
  }

  @Override
  protected androidx.media3.exoplayer.offline.DownloadManager getDownloadManager() {
    return OfflineStore.manager();
  }

  @Override
  protected Scheduler getScheduler() {
    return null;
  }

  @Override
  protected Notification getForegroundNotification(List<Download> downloads, int requirements) {
    return new Notification.Builder(this, "ytui-downloads")
        .setSmallIcon(dev.ytui.android.R.drawable.ic_ytui)
        .setContentTitle("ytui downloads")
        .setContentText(downloads.size() + " media entries")
        .setOngoing(true)
        .build();
  }
}
