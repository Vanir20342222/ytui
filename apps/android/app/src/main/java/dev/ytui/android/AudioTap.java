package dev.ytui.android;

import androidx.media3.common.C;
import androidx.media3.common.audio.BaseAudioProcessor;
import java.nio.ByteBuffer;

/** A pass-through tap; numerical analysis is Rust and never records audio. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class AudioTap extends BaseAudioProcessor {
  static volatile long levels;
  static volatile boolean enabled;

  @Override
  public AudioFormat onConfigure(AudioFormat input) {
    return input.encoding == C.ENCODING_PCM_16BIT ? input : AudioFormat.NOT_SET;
  }

  @Override
  public void queueInput(ByteBuffer input) {
    int size = input.remaining();
    if (enabled && size > 0)
      levels = NativeBridge.meter(input, input.position(), size, inputAudioFormat.channelCount);
    ByteBuffer output = replaceOutputBuffer(size);
    output.put(input);
    output.flip();
  }
}
