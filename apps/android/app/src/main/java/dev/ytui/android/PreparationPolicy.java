package dev.ytui.android;

/** Pure platform retry/checkpoint policy, kept testable without a device. */
final class PreparationPolicy {
  static boolean rateLimited(String error) {
    String lower = error.toLowerCase(java.util.Locale.ROOT);
    return lower.contains("429") || lower.contains("rate limit") || lower.contains("cooldown");
  }

  static int checkedCursor(int cursor, int count) {
    if (count < 0 || count > 2000 || cursor < 0 || cursor > count)
      throw new IllegalArgumentException("Invalid playlist checkpoint");
    return cursor;
  }

  private PreparationPolicy() {}
}
