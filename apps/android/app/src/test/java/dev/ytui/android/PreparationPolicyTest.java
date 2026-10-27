package dev.ytui.android;

import static org.junit.Assert.*;

import org.junit.Test;

public final class PreparationPolicyTest {
  @Test
  public void serviceCooldownNeverBecomesAnEntryFailure() {
    assertTrue(PreparationPolicy.rateLimited("HTTP 429"));
    assertTrue(PreparationPolicy.rateLimited("RATE LIMIT"));
    assertTrue(PreparationPolicy.rateLimited("Service cooldown"));
    assertFalse(PreparationPolicy.rateLimited("Video unavailable"));
  }

  @Test
  public void checkpointResumesEveryEntryThroughTheLast() {
    int cursor = 0;
    int count = 42;
    while (cursor < count) {
      cursor = PreparationPolicy.checkedCursor(cursor + 1, count);
    }
    assertEquals(count, cursor);
    assertEquals(0, PreparationPolicy.checkedCursor(0, 0));
  }

  @Test
  public void invalidCheckpointIsRejected() {
    for (int[] row : new int[][] {{-1, 2}, {3, 2}, {0, 2001}}) {
      try {
        PreparationPolicy.checkedCursor(row[0], row[1]);
        fail();
      } catch (IllegalArgumentException expected) {
      }
    }
  }
}
