How the System Works:

  1. Frame Generation & Streaming

  - No Rust FFmpeg library - Uses external FFmpeg processes via Command::new("ffmpeg")
  - One FFmpeg per camera for RTSP-to-MJPEG transcoding
  - FFmpeg reads from RTSP streams and outputs MJPEG frames to stdout
  - Frames are read from FFmpeg stdout and sent to a broadcast channel (Arc<broadcast::Sender<Bytes>>)

  2. Frame Storage - In Memory (Broadcast Channels)

  - Frames are NOT stored on disk initially
  - All MJPEG frames flow through broadcast channels in memory
  - Multiple subscribers can listen to the same frame stream:
    - WebSocket clients (live streaming to browsers)
    - Frame recording (database storage)
    - Video segmenter (MP4 creation)

  3. MP4 Creation Process

  When video storage is enabled:
  - Video segmenter loop (video_segmenter_loop) runs for each camera
  - Collects frames in a buffer (Vec<Bytes>) for the segment duration (default 5 minutes)
  - When segment time expires, calls create_video_segment() which:
    a. Spawns a new FFmpeg process specifically for MP4 creation
    b. Pipes MJPEG frames to FFmpeg stdin
    c. FFmpeg command: ffmpeg -f mjpeg -i - -c:v libx264 -preset ultrafast -y output.mp4
    d. Writes the MP4 file to the recordings directory

  4. Why You Don't See Additional FFmpeg Processes

  - The MP4 creation FFmpeg processes are short-lived (only during segment creation)
  - They spawn, process the buffered frames, create the MP4, then exit
  - You mainly see the persistent RTSP-to-MJPEG FFmpeg processes (one per camera)

  5. File Structure

  recordings/
  ├── cam1.db                    # Frame-by-frame SQLite database
  ├── cam1_1724058000.mp4       # Video segments
  ├── cam1_1724058300.mp4
  ├── cam2.db
  ├── cam2_1724058000.mp4
  └── ...

  Summary Architecture:

  RTSP Camera → FFmpeg (persistent) → MJPEG frames → Broadcast Channel → {
    ├── WebSocket clients (live streaming)
    ├── Frame recorder (SQLite database)
    └── Video segmenter → FFmpeg (temporary) → MP4 files
  }

  The system is quite efficient - it uses one persistent FFmpeg per camera for real-time streaming, and temporary FFmpeg processes only when creating MP4 segments from the buffered frames.
