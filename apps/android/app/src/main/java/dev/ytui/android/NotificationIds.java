package dev.ytui.android;

import java.util.UUID;

/** Keep WorkManager notifications away from the playback MediaStyle notification. */
final class NotificationIds {
  static final int DOWNLOADS = 2;
  static final int PLAYBACK = 4;

  static int preparation(UUID work) {
    return 0x10000000 | (work.hashCode() & 0x0fffffff);
  }

  static int export(UUID work) {
    return 0x20000000 | (work.hashCode() & 0x0fffffff);
  }

  private NotificationIds() {}
}
