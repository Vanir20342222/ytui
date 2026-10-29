package dev.ytui.android;

import android.app.*;
import android.content.*;
import android.content.pm.ServiceInfo;
import android.media.*;
import android.net.Uri;
import androidx.media3.common.C;
import androidx.media3.datasource.*;
import androidx.media3.exoplayer.offline.Download;
import androidx.work.*;
import java.io.*;
import java.nio.ByteBuffer;
import java.util.*;
import java.util.concurrent.TimeUnit;
import java.util.function.BooleanSupplier;
import org.json.*;

/** Platform storage/container adapter. Reads completed cache only; never contacts a server. */
@androidx.annotation.OptIn(markerClass = androidx.media3.common.util.UnstableApi.class)
public final class OfflineExport extends Worker {
  static final class Plan {
    final String title, extension, mime;
    final List<Download> parts;
    final int container;

    Plan(String title, String extension, String mime, List<Download> parts, int container) {
      this.title = title;
      this.extension = extension;
      this.mime = mime;
      this.parts = parts;
      this.container = container;
    }
  }

  public OfflineExport(Context context, WorkerParameters parameters) {
    super(context, parameters);
  }

  static java.util.UUID enqueue(String id, Uri destination) {
    OneTimeWorkRequest work =
        new OneTimeWorkRequest.Builder(OfflineExport.class)
            .setInputData(
                new Data.Builder()
                    .putString("download", id)
                    .putString("destination", destination.toString())
                    .build())
            .addTag("ytui-export")
            .build();
    state(MediaBridge.context, work.getId().toString(), "Export queued");
    WorkManager.getInstance(MediaBridge.context).enqueue(work);
    return work.getId();
  }

  static void enqueueAuto(String id) {
    if (!MobileSettings.enabled("auto_save_folders")) return;
    try {
      if (StorageLocations.record(id) != null) return;
    } catch (Exception invalid) {
      return;
    }
    OneTimeWorkRequest work =
        new OneTimeWorkRequest.Builder(OfflineExport.class)
            .setInputData(
                new Data.Builder().putString("download", id).putBoolean("auto", true).build())
            .addTag("ytui-folder-save")
            .build();
    WorkManager.getInstance(MediaBridge.context)
        .enqueueUniqueWork("ytui-folder-" + id, ExistingWorkPolicy.KEEP, work);
  }

  static void state(Context context, String id, String text) {
    if (context
        .getSharedPreferences("export-dismissed", Context.MODE_PRIVATE)
        .getBoolean(id, false)) return;
    context
        .getSharedPreferences("exports", Context.MODE_PRIVATE)
        .edit()
        .putString(id, text)
        .apply();
  }

  @Override
  public Result doWork() {
    String id = getInputData().getString("download"), raw = getInputData().getString("destination");
    boolean automatic = getInputData().getBoolean("auto", false);
    if (id == null || (raw == null && !automatic)) return Result.failure();
    Uri destination = raw == null ? null : Uri.parse(raw);
    try {
      JSONObject request = null;
      if (automatic) {
        if (!MobileSettings.enabled("auto_save_folders")
            || StorageLocations.prefs().getBoolean("removed:" + id, false)
            || StorageLocations.record(id) != null) return Result.success();
        request = OfflineStore.cachedRequest(id);
        Uri tree = StorageLocations.folder(request.optBoolean("audio"));
        if (tree == null) return Result.success();
        if (!StorageLocations.granted(tree))
          throw new IOException("Offline folder permission expired");
        Plan plan = inspect(id);
        destination =
            android.provider.DocumentsContract.createDocument(
                getApplicationContext().getContentResolver(),
                android.provider.DocumentsContract.buildDocumentUriUsingTree(
                    tree, android.provider.DocumentsContract.getTreeDocumentId(tree)),
                plan.mime,
                PlaybackPolicy.exportName(plan.title, plan.extension));
        if (destination == null) throw new IOException("Could not create offline media file");
      }
      setForegroundAsync(foreground()).get(15, TimeUnit.SECONDS);
      state(getApplicationContext(), getId().toString(), "Exporting file");
      write(
          id,
          destination,
          () ->
              isStopped()
                  || automatic && StorageLocations.prefs().getBoolean("removed:" + id, false));
      if (automatic) StorageLocations.remember(id, destination, request);
      state(getApplicationContext(), getId().toString(), "Export complete");
      MediaBridge.event("storage_updated", "");
      return Result.success();
    } catch (Exception error) {
      if (automatic) StorageLocations.failed(id);
      // Stack locations and exception class only: never log signed URLs or URI grants.
      android.util.Log.e(
          "YtuiExport", error.getClass().getName() + " " + Arrays.toString(error.getStackTrace()));
      state(
          getApplicationContext(),
          getId().toString(),
          isStopped()
              ? "Export cancelled"
              : "Export failed; choose a supported completed download or another destination");
      // ACTION_CREATE_DOCUMENT created this new output; remove a partial document on failure.
      try {
        if (destination != null)
          android.provider.DocumentsContract.deleteDocument(
              getApplicationContext().getContentResolver(), destination);
      } catch (Exception ignored) {
      }
      MediaBridge.event("error", "Export failed: " + safeReason(error));
      return Result.failure();
    }
  }

  static String safeReason(Exception error) {
    return error instanceof UnsupportedOperationException
        ? error.getMessage()
        : error instanceof InterruptedIOException
            ? "Cancelled"
            : "Cannot write the completed offline media; check storage and destination access";
  }

  static Plan inspect(String id) throws Exception {
    JSONObject request = OfflineStore.cachedRequest(id);
    Download main = OfflineStore.manager().getDownloadIndex().getDownload(id);
    List<Download> parts = new ArrayList<>();
    parts.add(main);
    String audio = request.getJSONObject("stream").optString("audio_url", "");
    if (!audio.isEmpty() && !"null".equals(audio))
      parts.add(OfflineStore.manager().getDownloadIndex().getDownload(id + "-audio"));
    List<String> mimes = new ArrayList<>();
    for (Download part : parts) {
      if (part == null || part.state != Download.STATE_COMPLETED || part.contentLength <= 0)
        throw new UnsupportedOperationException(
            "Export requires a complete progressive file; segmented/live downloads stay playable"
                + " inside ytui");
      try (CachedInput input = new CachedInput(part)) {
        MediaExtractor extractor = new MediaExtractor();
        try {
          extractor.setDataSource(input);
          if (extractor.getDrmInitData() != null)
            throw new UnsupportedOperationException("Encrypted media cannot be exported");
          for (int i = 0; i < extractor.getTrackCount(); i++) {
            String mime = extractor.getTrackFormat(i).getString(MediaFormat.KEY_MIME);
            if (mime != null && (mime.startsWith("audio/") || mime.startsWith("video/")))
              mimes.add(mime);
          }
        } finally {
          extractor.release();
        }
      }
    }
    if (mimes.isEmpty()) throw new UnsupportedOperationException("No exportable media tracks");
    boolean audioOnly = mimes.stream().noneMatch(m -> m.startsWith("video/"));
    // A single container already holds the original tracks: lossless byte copy.
    if (parts.size() == 1) {
      byte[] header = new byte[12];
      try (CachedInput input = new CachedInput(main)) {
        input.readAt(0, header, 0, header.length);
      }
      boolean mp4 = header[4] == 'f' && header[5] == 't' && header[6] == 'y' && header[7] == 'p';
      boolean webm =
          header[0] == 0x1a
              && header[1] == 0x45
              && (header[2] & 255) == 0xdf
              && (header[3] & 255) == 0xa3;
      if (!mp4 && !webm)
        throw new UnsupportedOperationException(
            "Original container export supports MP4/M4A and WebM downloads");
      String extension = mp4 ? (audioOnly ? "m4a" : "mp4") : "webm";
      return new Plan(
          request.getJSONObject("media").optString("title", "ytui-media"),
          extension,
          (audioOnly ? "audio/" : "video/") + (mp4 ? "mp4" : "webm"),
          parts,
          -1);
    }
    boolean webm =
        mimes.stream()
            .allMatch(
                m ->
                    Arrays.asList(
                            "video/x-vnd.on2.vp8",
                            "video/x-vnd.on2.vp9",
                            "audio/opus",
                            "audio/vorbis")
                        .contains(m));
    if (!webm
        && !mimes.stream()
            .allMatch(
                m ->
                    Arrays.asList("video/avc", "video/hevc", "video/av01", "audio/mp4a-latm")
                        .contains(m)))
      throw new UnsupportedOperationException(
          "This codec combination cannot be exported by Android; choose another download quality");
    return new Plan(
        request.getJSONObject("media").optString("title", "ytui-media"),
        webm ? "webm" : "mp4",
        webm ? "video/webm" : "video/mp4",
        parts,
        webm
            ? MediaMuxer.OutputFormat.MUXER_OUTPUT_WEBM
            : MediaMuxer.OutputFormat.MUXER_OUTPUT_MPEG_4);
  }

  static void write(String id, Uri destination, BooleanSupplier cancelled) throws Exception {
    Plan plan = inspect(id);
    check(cancelled);
    File staged = null;
    try {
      if (plan.container >= 0) {
        long needed = 16L * 1024 * 1024;
        for (Download part : plan.parts) needed = Math.addExact(needed, part.contentLength);
        if (MediaBridge.context.getCacheDir().getUsableSpace() < needed)
          throw new IOException("Not enough temporary space");
        staged =
            File.createTempFile(
                "ytui-export-", "." + plan.extension, MediaBridge.context.getCacheDir());
        mux(plan, staged, cancelled);
      }
      try (OutputStream output =
          MediaBridge.context.getContentResolver().openOutputStream(destination, "w")) {
        if (output == null) throw new IOException("No output stream");
        if (staged != null) {
          try (InputStream input = new FileInputStream(staged)) {
            byte[] buffer = new byte[65536];
            int count;
            while ((count = input.read(buffer)) != -1) {
              check(cancelled);
              output.write(buffer, 0, count);
            }
          }
        } else
          try (CachedInput input = new CachedInput(plan.parts.get(0))) {
            byte[] buffer = new byte[65536];
            long position = 0;
            int count;
            while ((count = input.readAt(position, buffer, 0, buffer.length)) != -1) {
              check(cancelled);
              if (count == 0) throw new IOException("Offline cache stalled");
              output.write(buffer, 0, count);
              position += count;
            }
            if (position != input.getSize()) throw new EOFException("Offline cache is incomplete");
          }
      }
    } finally {
      if (staged != null) staged.delete();
    }
  }

  private static void check(BooleanSupplier cancelled) throws InterruptedIOException {
    if (cancelled.getAsBoolean() || Thread.currentThread().isInterrupted())
      throw new InterruptedIOException();
  }

  static void mux(Plan plan, File output, BooleanSupplier cancelled) throws Exception {
    List<CachedInput> inputs = new ArrayList<>();
    List<MediaExtractor> extractors = new ArrayList<>();
    List<int[]> maps = new ArrayList<>();
    MediaMuxer muxer = new MediaMuxer(output.getAbsolutePath(), plan.container);
    boolean started = false;
    int trackCount = 0;
    Set<Integer> populatedTracks = new HashSet<>();
    try {
      for (Download part : plan.parts) {
        CachedInput input = new CachedInput(part);
        inputs.add(input);
        MediaExtractor extractor = new MediaExtractor();
        extractors.add(extractor);
        extractor.setDataSource(input);
        int[] map = new int[extractor.getTrackCount()];
        Arrays.fill(map, -1);
        for (int i = 0; i < map.length; i++) {
          MediaFormat format = extractor.getTrackFormat(i);
          String mime = format.getString(MediaFormat.KEY_MIME);
          if (mime != null && (mime.startsWith("audio/") || mime.startsWith("video/"))) {
            map[i] = muxer.addTrack(format);
            trackCount++;
            extractor.selectTrack(i);
            if (mime.startsWith("video/") && format.containsKey(MediaFormat.KEY_ROTATION))
              muxer.setOrientationHint(format.getInteger(MediaFormat.KEY_ROTATION));
          }
        }
        maps.add(map);
      }
      muxer.start();
      started = true;
      ByteBuffer buffer = ByteBuffer.allocateDirect(8 * 1024 * 1024);
      MediaCodec.BufferInfo info = new MediaCodec.BufferInfo();
      long origin = 0;
      for (MediaExtractor extractor : extractors) {
        if (extractor.getSampleTrackIndex() >= 0)
          origin = Math.min(origin, extractor.getSampleTime());
      }
      while (true) {
        check(cancelled);
        int next = -1;
        long timestamp = Long.MAX_VALUE;
        for (int i = 0; i < extractors.size(); i++) {
          long time = extractors.get(i).getSampleTime();
          // AAC priming samples have valid negative timestamps. Only the track
          // index signals EOS; dropping priming must not drop the entire track.
          if (extractors.get(i).getSampleTrackIndex() >= 0 && time < timestamp) {
            next = i;
            timestamp = time;
          }
        }
        if (next < 0) break;
        MediaExtractor extractor = extractors.get(next);
        if (android.os.Build.VERSION.SDK_INT >= 28 && extractor.getSampleSize() > buffer.capacity())
          throw new IOException("Sample exceeds export buffer");
        buffer.clear();
        int size = extractor.readSampleData(buffer, 0);
        if (size < 0) throw new EOFException("Missing cached sample");
        int flags =
            (extractor.getSampleFlags() & MediaExtractor.SAMPLE_FLAG_SYNC) != 0
                ? MediaCodec.BUFFER_FLAG_KEY_FRAME
                : 0;
        if ((extractor.getSampleFlags() & MediaExtractor.SAMPLE_FLAG_ENCRYPTED) != 0)
          throw new IOException("Encrypted sample");
        info.set(0, size, timestamp - origin, flags);
        populatedTracks.add(maps.get(next)[extractor.getSampleTrackIndex()]);
        muxer.writeSampleData(maps.get(next)[extractor.getSampleTrackIndex()], buffer, info);
        extractor.advance();
      }
      if (populatedTracks.size() != trackCount)
        throw new IOException("An exported track has no samples");
      muxer.stop();
      started = false;
    } finally {
      if (started)
        try {
          muxer.stop();
        } catch (RuntimeException ignored) {
        }
      try {
        muxer.release();
      } finally {
        for (MediaExtractor extractor : extractors) extractor.release();
        for (CachedInput input : inputs)
          try {
            input.close();
          } catch (IOException ignored) {
          }
      }
    }
  }

  static final class CachedInput extends MediaDataSource {
    private final DataSource source = OfflineStore.offlineFactory().createDataSource();
    private final Download download;
    private long position = -1;
    private boolean opened;

    CachedInput(Download download) {
      this.download = download;
    }

    @Override
    public synchronized int readAt(long offset, byte[] buffer, int start, int size)
        throws IOException {
      if (offset < 0) throw new IOException("Negative offset");
      if (size == 0) return 0;
      if (offset >= download.contentLength) return -1;
      if (position != offset) {
        if (opened) {
          source.close();
          opened = false;
        }
        source.open(
            new DataSpec.Builder()
                .setUri(download.request.uri)
                .setKey(download.request.customCacheKey)
                .setPosition(offset)
                .build());
        opened = true;
        position = offset;
      }
      int count = source.read(buffer, start, (int) Math.min(size, download.contentLength - offset));
      if (count == C.RESULT_END_OF_INPUT) return -1;
      if (count > 0) position += count;
      return count;
    }

    @Override
    public long getSize() {
      return download.contentLength;
    }

    @Override
    public synchronized void close() throws IOException {
      if (opened) {
        opened = false;
        source.close();
      }
    }
  }

  private ForegroundInfo foreground() {
    NotificationManager manager =
        getApplicationContext().getSystemService(NotificationManager.class);
    manager.createNotificationChannel(
        new NotificationChannel(
            "ytui-exports", "Media exports", NotificationManager.IMPORTANCE_LOW));
    Notification notification =
        new Notification.Builder(getApplicationContext(), "ytui-exports")
            .setSmallIcon(R.drawable.ic_ytui)
            .setContentTitle("ytui: exporting offline media")
            .setContentText("Writing the completed file to your chosen destination")
            .setOngoing(true)
            .build();
    return android.os.Build.VERSION.SDK_INT >= 29
        ? new ForegroundInfo(
            NotificationIds.export(getId()),
            notification,
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        : new ForegroundInfo(NotificationIds.export(getId()), notification);
  }
}
