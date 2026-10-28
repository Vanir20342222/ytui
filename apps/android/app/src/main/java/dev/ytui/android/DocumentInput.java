package dev.ytui.android;

import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;

/** Scoped document bytes only; parsing and application decisions stay in Rust. */
final class DocumentInput {
  static String read(InputStream input, int limit) throws IOException {
    if (input == null || limit < 0) throw new IOException("No document data");
    ByteArrayOutputStream bytes = new ByteArrayOutputStream();
    byte[] buffer = new byte[4096];
    int count;
    while ((count = input.read(buffer)) != -1) {
      if (count == 0) throw new IOException("Document stream made no progress");
      if (count > limit - bytes.size()) throw new IOException("Document exceeds size limit");
      bytes.write(buffer, 0, count);
    }
    return StandardCharsets.UTF_8
        .newDecoder()
        .decode(ByteBuffer.wrap(bytes.toByteArray()))
        .toString();
  }

  private DocumentInput() {}
}
