package dev.ytui.android;

import static org.junit.Assert.*;

import java.io.*;
import java.nio.charset.StandardCharsets;
import org.junit.Test;

public final class DocumentInputTest {
  @Test
  public void unicodeIsPreservedAtTheExactByteLimit() throws Exception {
    byte[] data = "[00:01.234]Grüße 🎵".getBytes(StandardCharsets.UTF_8);
    assertEquals(
        "[00:01.234]Grüße 🎵", DocumentInput.read(new ByteArrayInputStream(data), data.length));
    assertThrows(
        IOException.class,
        () -> DocumentInput.read(new ByteArrayInputStream(data), data.length - 1));
  }

  @Test
  public void malformedUtf8CannotSilentlyChangeCookieOrLyricText() {
    for (byte[] data : new byte[][] {{(byte) 0xff}, {(byte) 0xe2, (byte) 0x82}}) {
      assertThrows(IOException.class, () -> DocumentInput.read(new ByteArrayInputStream(data), 32));
    }
  }

  @Test
  public void unavailableAndNonProgressingStreamsFail() {
    assertThrows(IOException.class, () -> DocumentInput.read(null, 32));
    assertThrows(
        IOException.class,
        () ->
            DocumentInput.read(
                new InputStream() {
                  public int read() {
                    return 0;
                  }

                  public int read(byte[] buffer) {
                    return 0;
                  }
                },
                32));
  }
}
