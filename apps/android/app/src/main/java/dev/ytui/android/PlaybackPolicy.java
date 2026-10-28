package dev.ytui.android;

/** The Rust MediaKind JSON contract uses lowercase names. */
final class PlaybackPolicy {
  static boolean audioOnly(String kind) {
    return "song".equals(kind) || "station".equals(kind);
  }

  static String liveTitle(CharSequence title) {
    if (title == null) return "";
    return title
        .toString()
        .codePoints()
        .limit(512)
        .map(cp -> Character.isISOControl(cp) ? ' ' : cp >= 0xd800 && cp <= 0xdfff ? 0xfffd : cp)
        .collect(StringBuilder::new, StringBuilder::appendCodePoint, StringBuilder::append)
        .toString()
        .trim();
  }

  static String exportName(String title, String extension) {
    String clean = title.replaceAll("[\\p{Cntrl}/\\\\:*?\"<>|]", "_").trim();
    if (clean.isEmpty()) clean = "ytui-media";
    clean =
        clean
            .codePoints()
            .limit(100)
            .collect(StringBuilder::new, StringBuilder::appendCodePoint, StringBuilder::append)
            .toString();
    return clean + "." + extension;
  }

  private PlaybackPolicy() {}
}
