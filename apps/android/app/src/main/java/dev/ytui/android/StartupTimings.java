package dev.ytui.android;

import java.util.ArrayDeque;
import java.util.concurrent.atomic.AtomicLong;

/** Local, bounded, identifier-free tap-to-output measurements. All times are monotonic. */
final class StartupTimings {
  private static final AtomicLong ids = new AtomicLong();
  private static final ArrayDeque<Sample> samples = new ArrayDeque<>();

  private static final class Sample {
    long id, tap, resolved = -1, handoff = -1, output = -1;
    boolean prepared, video;

    Sample(long id, long tap, boolean video) {
      this.id = id;
      this.tap = tap;
      this.video = video;
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
        && now >= sample.tap
        && sample.output < 0) sample.output = now - sample.tap;
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
          .append(": resolve=")
          .append(sample.resolved)
          .append(", handoff=")
          .append(sample.handoff)
          .append(sample.video ? ", visible_frame=" : ", playing=")
          .append(sample.output)
          .append(
              sample.output < 0
                  ? " pending/cancelled"
                  : sample.video && sample.output > 2000 ? " OVER TARGET" : "")
          .append('\n');
    }
    return text.toString();
  }

  private StartupTimings() {}
}
