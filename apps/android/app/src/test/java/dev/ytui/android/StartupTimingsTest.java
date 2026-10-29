package dev.ytui.android;

import static org.junit.Assert.*;

import org.junit.Test;

public final class StartupTimingsTest {
  @Test
  public void totalIncludesExtractionAndCannotCountAudioAsVideo() {
    long token = StartupTimings.begin(1000, true);
    StartupTimings.resolved(token, 2400, false);
    StartupTimings.route(token, "https://should-not-be-reported.invalid");
    assertFalse(StartupTimings.report().contains("should-not-be-reported"));
    StartupTimings.route(token, "direct");
    assertTrue(StartupTimings.report().contains("route=direct"));
    StartupTimings.output(token, 2450, true);
    assertEquals(-1, StartupTimings.total(token)); // no player handoff yet
    StartupTimings.handoff(token, 2500);
    StartupTimings.output(token, 2600, false);
    assertEquals(-1, StartupTimings.total(token));
    StartupTimings.output(token, 3200, true);
    assertEquals(2200, StartupTimings.total(token));
    StartupTimings.output(token, 9000, true); // fullscreen doesn't replace the first result
    assertEquals(2200, StartupTimings.total(token));
    assertTrue(StartupTimings.report().contains("OVER TARGET"));
  }

  @Test
  public void replacedRequestsRemainIndependentAndReportsAreBounded() {
    long old = StartupTimings.begin(1000, true);
    long next = StartupTimings.begin(2000, false);
    StartupTimings.resolved(next, 2010, true);
    StartupTimings.handoff(next, 2020);
    StartupTimings.output(next, 2200, false);
    assertEquals(200, StartupTimings.total(next));
    assertEquals(-1, StartupTimings.total(old));
    for (int i = 0; i < 30; i++) StartupTimings.begin(3000 + i, true);
    assertEquals(-1, StartupTimings.total(next));
    assertEquals(20, StartupTimings.report().split("pending/cancelled", -1).length - 1);
    assertFalse(StartupTimings.report().contains("http"));
  }
}
