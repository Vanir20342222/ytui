package dev.ytui.android;

import android.os.ParcelFileDescriptor;
import androidx.javascriptengine.*;
import com.google.common.util.concurrent.ListenableFuture;
import java.io.*;
import java.nio.charset.StandardCharsets;
import java.util.concurrent.*;

/** Runs only offline player challenge batches, isolated from cookies, UI and network. */
public final class ScriptRuntime {
  private static ListenableFuture<JavaScriptSandbox> connection;
  private static final Semaphore slots = new Semaphore(2);

  private static synchronized ListenableFuture<JavaScriptSandbox> connection() {
    if (connection == null)
      connection = JavaScriptSandbox.createConnectedInstanceAsync(MediaBridge.context);
    return connection;
  }

  public static String evaluate(String script) {
    final long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(15);
    JavaScriptIsolate isolate = null;
    File file = null;
    boolean acquired = false;
    try {
      if (script.length() > 16 * 1024 * 1024) throw new IOException("JS input limit");
      acquired = slots.tryAcquire(remaining(deadline), TimeUnit.MILLISECONDS);
      if (!acquired) throw new TimeoutException();
      JavaScriptSandbox sandbox = connection().get(remaining(deadline), TimeUnit.MILLISECONDS);
      if (!sandbox.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_EVALUATE_FROM_FD)
          || !sandbox.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_ISOLATE_MAX_HEAP_SIZE))
        throw new IOException("Update Android System WebView for isolated YouTube solving");
      IsolateStartupParameters limits = new IsolateStartupParameters();
      if (sandbox.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_ISOLATE_MAX_HEAP_SIZE))
        limits.setMaxHeapSizeBytes(128L * 1024 * 1024);
      if (sandbox.isFeatureSupported(
          JavaScriptSandbox.JS_FEATURE_EVALUATE_WITHOUT_TRANSACTION_LIMIT))
        limits.setMaxEvaluationReturnSizeBytes(1024 * 1024);
      isolate = sandbox.createIsolate(limits);
      String executable =
          "(()=>{let result='';const console={log:(value)=>{result=String(value);}};\n"
              + script
              + "\nreturn result;})()";
      file = File.createTempFile("ytui-solver-", ".js", MediaBridge.context.getCacheDir());
      try (FileOutputStream out = new FileOutputStream(file)) {
        out.write(executable.getBytes(StandardCharsets.UTF_8));
      }
      try (ParcelFileDescriptor descriptor =
          ParcelFileDescriptor.open(file, ParcelFileDescriptor.MODE_READ_ONLY)) {
        if (!sandbox.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_EVALUATE_FROM_FD))
          throw new IOException("FD evaluation unavailable");
        String result;
        if (sandbox.isFeatureSupported(JavaScriptSandbox.JS_FEATURE_EVALUATE_FROM_FD)) {
          result =
              isolate
                  .evaluateJavaScriptAsync(descriptor)
                  .get(remaining(deadline), TimeUnit.MILLISECONDS);
        } else throw new IOException("FD evaluation unavailable");
        if (result.length() > 1024 * 1024) throw new IOException("JS output limit");
        return result;
      }
    } catch (Exception error) {
      throw new IllegalStateException(
          "Isolated YouTube solver unavailable or timed out; update Android System WebView. Main"
              + " thread: "
              + java.util.Arrays.toString(
                  android.os.Looper.getMainLooper().getThread().getStackTrace()),
          error);
    } finally {
      if (isolate != null) isolate.close();
      if (file != null && !file.delete()) file.deleteOnExit();
      if (acquired) slots.release();
    }
  }

  private static long remaining(long deadline) throws TimeoutException {
    long ms = TimeUnit.NANOSECONDS.toMillis(deadline - System.nanoTime());
    if (ms <= 0) throw new TimeoutException();
    return ms;
  }

  private ScriptRuntime() {}
}
