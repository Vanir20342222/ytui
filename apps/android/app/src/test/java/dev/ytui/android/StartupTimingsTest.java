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
    assertEquals(1700, StartupTimings.elapsed(token, 2700));
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

  @Test
  public void joinedPreloadIsNotReportedAsReadyAndDiagnosticsAreAllowlisted() {
    long token = StartupTimings.begin(1000, true);
    StartupTimings.resolved(token, 11000, true);
    StartupTimings.cacheState(token, "joined");
    StartupTimings.stage(token, "flight_wait", 9900);
    StartupTimings.stage(token, "https://private.invalid", 1);
    StartupTimings.stage(token, "extract", -10);
    StartupTimings.stage(token, "manifest", 120);
    StartupTimings.presentation(token, "best", "hls-chunkless");
    String report = StartupTimings.report();
    assertTrue(report.contains("Video cold route=unknown: resolve=10000"));
    assertTrue(report.contains("cache=joined flight_wait_ms=9900"));
    assertFalse(report.contains("private.invalid"));
    assertFalse(report.contains("extract_ms=-10"));
    assertTrue(report.contains("manifest_ms=120"));
    assertTrue(report.contains("quality=best transport=hls-chunkless"));
    StartupTimings.presentation(token, "https://private.invalid", "secret-token");
    assertFalse(StartupTimings.report().contains("private.invalid"));
    assertFalse(StartupTimings.report().contains("secret-token"));
  }

  @Test
  public void failuresAndCancellationRemainVisibleWithoutReplacingCompletedOutput() {
    long failed = StartupTimings.begin(1000, true);
    StartupTimings.ended(failed, "failed", 1300);
    long cancelled = StartupTimings.begin(2000, false);
    StartupTimings.ended(cancelled, "cancelled", 2400);
    String report = StartupTimings.report();
    assertTrue(report.contains("failed cache=unknown ended_ms=300"));
    assertTrue(report.contains("cancelled cache=unknown ended_ms=400"));
    StartupTimings.handoff(cancelled, 2500);
    StartupTimings.output(cancelled, 2600, false);
    assertEquals(-1, StartupTimings.total(cancelled));
    long completed = StartupTimings.begin(3000, false);
    StartupTimings.handoff(completed, 3100);
    StartupTimings.output(completed, 3200, false);
    StartupTimings.ended(completed, "failed", 4000);
    assertEquals(200, StartupTimings.total(completed));
  }
}
