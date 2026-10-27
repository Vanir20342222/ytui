package dev.ytui.android;

/** JNI has no UI policy; all screens, themes and actions live in Rust. */
public final class NativeBridge {
  static {
    System.loadLibrary("ytui_mobile");
  }

  private NativeBridge() {}

  public static native String init(String files, String cache);

  public static native String request(String json);

  public static native void cancel(long id);

  public static native void frameAvailable();

  public static native long meter(java.nio.ByteBuffer buffer, int offset, int length, int channels);
}
