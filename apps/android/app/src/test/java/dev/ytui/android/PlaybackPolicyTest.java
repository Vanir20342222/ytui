package dev.ytui.android;

import static org.junit.Assert.*;

import org.junit.Test;

public final class PlaybackPolicyTest {
  @Test
  public void rustSongAndStationQueueEntriesRemainAudioOnly() {
    assertTrue(PlaybackPolicy.audioOnly("song"));
    assertTrue(PlaybackPolicy.audioOnly("station"));
    for (String kind : new String[] {"video", "playlist", "album", "artist", "channel", ""})
      assertFalse(PlaybackPolicy.audioOnly(kind));
  }

  @Test
  public void liveMetadataIsBoundedWithoutControlCodesOrBrokenUnicode() {
    assertEquals("First second", PlaybackPolicy.liveTitle("First\nsecond\t\033"));
    String title = PlaybackPolicy.liveTitle("🎵".repeat(600));
    assertEquals(512, title.codePointCount(0, title.length()));
    assertFalse(Character.isHighSurrogate(title.charAt(title.length() - 1)));
    assertEquals("\ufffd", PlaybackPolicy.liveTitle("\ud800"));
    assertEquals("", PlaybackPolicy.liveTitle(null));
  }

  @Test
  public void exportNamesCannotBecomePathsOrTruncateUnicodeSurrogates() {
    assertEquals("a_b_c_.mp4", PlaybackPolicy.exportName("a/b\\c\n", "mp4"));
    String longName = PlaybackPolicy.exportName("🎵".repeat(120), "webm");
    assertEquals(100, longName.substring(0, longName.length() - 5).codePointCount(0, 200));
    assertEquals("ytui-media.m4a", PlaybackPolicy.exportName(" ", "m4a"));
  }
}
