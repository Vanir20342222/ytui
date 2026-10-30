package dev.ytui.android;

import java.util.ArrayDeque;
import java.util.concurrent.atomic.AtomicLong;

/** Local, bounded, identifier-free tap-to-output measurements. All times are monotonic. */
final class StartupTimings {
  static final String[] PHASES = {
    "selection",
    "offline",
    "rpc_queue",
    "engine",
    "flight_wait",
    "slot",
    "worker",
    "client",
    "extract",
    "visitor",
    "player",
    "manifest",
    "stream",
    "decoder",
    "ready",
    "net_response",
    "net_byte"
  };
  private static final AtomicLong ids = new AtomicLong();
  private static final ArrayDeque<Sample> samples = new ArrayDeque<>();

  private static final class Sample {
    long id, tap, resolved = -1, handoff = -1, output = -1;
    boolean prepared, video;
    String route = "unknown", cache = "unknown", disposition = "pending/cancelled";
    String quality = "unknown", transport = "unknown";
    long ended = -1;
    long[] stages = new long[PHASES.length];

    Sample(long id, long tap, boolean video) {
      this.id = id;
      this.tap = tap;
      this.video = video;
      java.util.Arrays.fill(stages, -1);
    }
  }

  static synchronized long begin(long now, boolean video) {
    long id = ids.incrementAndGet();
    samples.addLast(new Sample(id, now, video));
    while (samples.size() > 20) samples.removeFirst();
    return id;
  }

  private static Sample find(long id) {
    for (Sample sample : samples) if (sample.id == id) return sample;
    return null;
  }

  static synchronized void resolved(long id, long now, boolean prepared) {
    Sample sample = find(id);
    if (sample != null && now >= sample.tap && sample.resolved < 0) {
      sample.resolved = now - sample.tap;
      sample.prepared = prepared;
    }
  }

  static synchronized void route(long id, String route) {
    Sample sample = find(id);
    if (sample != null)
      sample.route =
          java.util.Arrays.asList("direct", "full", "offline", "direct-media").contains(route)
              ? route
              : "unknown";
  }

  static synchronized void presentation(long id, String quality, String transport) {
    Sample sample = find(id);
    if (sample == null) return;
    sample.quality =
        java.util.Arrays.asList("360", "480", "720", "1080", "best").contains(quality)
            ? quality
            : "unknown";
    sample.transport =
        java.util.Arrays.asList("hls-chunkless", "hls", "dash", "progressive", "offline")
                .contains(transport)
            ? transport
            : "unknown";
  }

  static synchronized void stage(long id, String phase, long milliseconds) {
    Sample sample = find(id);
    int index = java.util.Arrays.asList(PHASES).indexOf(phase);
    if (sample != null
        && index >= 0
        && sample.stages[index] < 0
        && milliseconds >= 0
        && milliseconds <= 3600000) sample.stages[index] = milliseconds;
  }

  static synchronized void cacheState(long id, String source) {
    Sample sample = find(id);
    if (sample == null || !java.util.Arrays.asList("ready", "joined", "miss").contains(source))
      return;
    sample.cache = source;
    // Joining a still-running preload is not a stream prepared before the tap.
    sample.prepared = "ready".equals(source);
  }

  static synchronized void ended(long id, String disposition, long now) {
    Sample sample = find(id);
    if (sample != null
        && sample.output < 0
        && sample.ended < 0
        && now >= sample.tap
        && java.util.Arrays.asList("failed", "cancelled").contains(disposition)) {
      sample.disposition = disposition;
      sample.ended = now - sample.tap;
    }
  }

  static synchronized void mediaType(long id, boolean video) {
    Sample sample = find(id);
    if (sample != null && sample.handoff < 0) sample.video = video;
  }

  static synchronized void handoff(long id, long now) {
    Sample sample = find(id);
    if (sample != null && now >= sample.tap && sample.handoff < 0)
      sample.handoff = now - sample.tap;
  }

  static synchronized void output(long id, long now, boolean video) {
    Sample sample = find(id);
    if (sample != null
        && sample.video == video
        && sample.handoff >= 0
        && sample.ended < 0
        && now >= sample.tap
        && sample.output < 0) sample.output = now - sample.tap;
  }

  static synchronized long elapsed(long id, long now) {
    Sample sample = find(id);
    return sample == null || now < sample.tap ? -1 : now - sample.tap;
  }

  static synchronized long total(long id) {
    Sample sample = find(id);
    return sample == null ? -1 : sample.output;
  }

  static synchronized String report() {
    StringBuilder text =
        new StringBuilder("ytui startup timings (milliseconds)\n")
            .append("Target: video tap to visible frame <= 2000 ms\n")
            .append(
                "Resolve includes offline lookup and queue selection. Pending attempts are"
                    + " included.\n");
    for (Sample sample : samples) {
      text.append(sample.video ? "Video" : "Audio")
          .append(sample.prepared ? " prepared" : " cold")
          .append(" route=")
          .append(sample.route)
          .append(": resolve=")
          .append(sample.resolved)
          .append(", handoff=")
          .append(sample.handoff)
          .append(sample.video ? ", visible_frame=" : ", playing=")
          .append(sample.output)
          .append(
              sample.output < 0
                  ? " " + sample.disposition
                  : sample.video && sample.output > 2000 ? " OVER TARGET" : "")
          .append(" cache=")
          .append(sample.cache);
      if (sample.ended >= 0) text.append(" ended_ms=").append(sample.ended);
      for (int i = 0; i < PHASES.length; i++)
        if (sample.stages[i] >= 0)
          text.append(" ").append(PHASES[i]).append("_ms=").append(sample.stages[i]);
      text.append(" quality=")
          .append(sample.quality)
          .append(" transport=")
          .append(sample.transport);
      text.append('\n');
    }
    return text.toString();
  }

  private StartupTimings() {}
}
