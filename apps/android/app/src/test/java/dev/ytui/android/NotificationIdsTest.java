package dev.ytui.android;

import static org.junit.Assert.*;

import java.util.UUID;
import org.junit.Test;

public final class NotificationIdsTest {
  @Test
  public void concurrentJobTypesCannotReplaceThePlaybackNotification() {
    UUID first = new UUID(0, 1), second = new UUID(0, 2);
    for (UUID work : new UUID[] {first, second}) {
      assertNotEquals(NotificationIds.PLAYBACK, NotificationIds.export(work));
      assertNotEquals(NotificationIds.PLAYBACK, NotificationIds.preparation(work));
      assertNotEquals(NotificationIds.DOWNLOADS, NotificationIds.export(work));
      assertNotEquals(NotificationIds.preparation(work), NotificationIds.export(work));
    }
    assertNotEquals(NotificationIds.export(first), NotificationIds.export(second));
    assertNotEquals(NotificationIds.preparation(first), NotificationIds.preparation(second));
  }
}
