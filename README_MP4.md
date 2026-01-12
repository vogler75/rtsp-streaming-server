How the MP4 Recording System Works:

  Configuration:
  
  The MP4 recording system is part of the dual-format recording architecture and can be configured independently:
  
  ```json
  {
    "recording": {
      "frame_storage_enabled": true,      // SQLite frame storage
      "video_storage_enabled": true,      // MP4 video segment creation
      "database_path": "recordings",
      "mp4_segment_minutes": 5,         // Duration of each MP4 segment
      "mp4_storage_retention": "30d"    // Auto-cleanup after 30 days
    }
  }
  ```

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
    d. Writes the MP4 file to the hierarchical recordings directory (recordings/{camera_id}/{YYYY}/{MM}/{DD}/)

  4. Why You Don't See Additional FFmpeg Processes

  - The MP4 creation FFmpeg processes are short-lived (only during segment creation)
  - They spawn, process the buffered frames, create the MP4, then exit
  - You mainly see the persistent RTSP-to-MJPEG FFmpeg processes (one per camera)

  5. File Structure

  The system now uses a hierarchical directory structure for better organization:

  recordings/
  ├── cam1.db                                    # Frame-by-frame SQLite database
  ├── cam1/                                      # MP4 video segments directory
  │   ├── 2024/
  │   │   └── 08/
  │   │       └── 19/
  │   │           ├── cam1_20240819_140000.mp4   # 5-minute segments with ISO 8601 format
  │   │           ├── cam1_20240819_140500.mp4
  │   │           └── cam1_20240819_141000.mp4
  ├── cam2.db                                    # Frame database for camera 2
  ├── cam2/                                      # MP4 segments for camera 2
  │   └── 2024/
  │       └── 08/
  │           └── 19/
  │               └── cam2_20240819_140000.mp4
  └── ...

  File Naming Convention:
  - SQLite databases: {camera_id}.db
  - MP4 segments: {camera_id}_{YYYYMMDD}_{HHMMSS}.mp4 (ISO 8601 format)
  - Directory structure: recordings/{camera_id}/{YYYY}/{MM}/{DD}/

  Benefits of hierarchical structure:
  - Easy navigation by date
  - Efficient cleanup of old recordings
  - Reduced directory listing overhead
  - Better file system performance with many files

  Summary Architecture:

  RTSP Camera → FFmpeg (persistent) → MJPEG frames → Broadcast Channel → {
    ├── WebSocket clients (live streaming)
    ├── Frame recorder (SQLite database)
    └── Video segmenter → FFmpeg (temporary) → MP4 files
  }

  The system is quite efficient - it uses one persistent FFmpeg per camera for real-time streaming, and temporary FFmpeg processes only when creating MP4 segments from the buffered frames.

  6. Automatic Cleanup

  The hierarchical directory structure enables efficient cleanup of old MP4 recordings:
  
  - Cleanup process scans the directory structure: recordings/{camera_id}/{YYYY}/{MM}/{DD}/
  - Deletes MP4 files older than the configured retention period based on filename timestamp
  - Removes empty directories after file deletion
  - Each camera's files are processed independently
  - Cleanup runs every `cleanup_interval_minutes` (default: 60 minutes)
  
  Example cleanup process:
  1. Check mp4_storage_retention (e.g., "30d")
  2. Calculate cutoff date (30 days ago)
  3. Scan directory structure for each camera
  4. Delete files older than cutoff: cam1_20240719_*.mp4 (if today is 2024-08-19)
  5. Remove empty date directories: recordings/cam1/2024/07/19/ (if empty)
  6. Log cleanup results: "Deleted 144 video segments for cam1"
