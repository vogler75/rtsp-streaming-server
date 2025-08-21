# MP4 Segment Streaming Feature Design

## Overview

This document outlines the design for adding MP4 segment streaming functionality to the RTSP streaming server. The feature enables playback of recorded content using a unified frame cache system that combines live recording buffers with on-demand MP4 segment conversion, completely bypassing the `recording_mjpeg` table for improved performance.

## Problem Statement

The current system queries the `recording_mjpeg` table for every frame during playback, which is inefficient. The system needs to:
- **Eliminate database queries** during playback by using in-memory frame caches
- **Unify frame sources**: Live recording buffer + converted MP4 segments in one cache
- Maintain instant `goto` functionality 
- Support smooth streaming playback
- Minimize memory usage while maximizing responsiveness

## Architecture Overview

### Current System Components

- **Frame Recording**: MJPEG frames stored in `recording_mjpeg` table
- **MP4 Segments**: 5-minute MP4 segments stored in `recording_mp4` table (filesystem or database)
- **Streaming Playback**: `FrameStream` trait provides frame iteration for WebSocket streaming
- **Goto Functionality**: `get_frame_at_timestamp()` for instant frame seeking

### New Components

```
┌─────────────────┐    ┌──────────────────┐    
│   WebSocket     │───▶│  RecordingManager│    
│   Controller    │    │                  │    
└─────────────────┘    └──────────────────┘    
                                │                         
                                ▼                         
                       ┌──────────────────────────────────┐
                       │    UnifiedFrameCache             │
                       │ ┌──────────────────────────────┐ │
                       │ │ Live Recording Buffer        │ │
                       │ │ (Current 5-min from recorder)│ │
                       │ └──────────────────────────────┘ │
                       │ ┌──────────────────────────────┐ │
                       │ │ MP4 Conversion Cache         │ │
                       │ │ (Historical 5-min windows)   │ │
                       │ └──────────────────────────────┘ │
                       └──────────────────────────────────┘
                                │                         
                                ▼                         
                       ┌──────────────────┐    ┌─────────────────┐
                       │ FFmpeg Converter │    │ CachedFrameStream│
                       │ (On-demand)      │    │ (No DB access)  │
                       └──────────────────┘    └─────────────────┘
```

**NO DATABASE ACCESS DURING PLAYBACK** - All frames served from memory cache

## Detailed Design

### 1. Unified Cache Strategy

**Lookup Chain**: `Live Recording Buffer → MP4 Conversion Cache → On-Demand MP4 Conversion`

**NO DATABASE ACCESS** - The `recording_mjpeg` table is never queried during playback.

```rust
async fn get_frame_at_timestamp(&self, camera_id: &str, timestamp: DateTime<Utc>) -> Result<Option<RecordedFrame>> {
    // 1. Check live recording buffer (frames from current recording)
    if let Some(frame) = self.get_frame_from_live_buffer(camera_id, timestamp).await {
        return Ok(Some(frame));
    }
    
    // 2. Check MP4 conversion cache (historical frames)
    let window_id = calculate_5min_window_id(timestamp);
    if let Some(cached_frame) = self.get_cached_mp4_frame(camera_id, window_id, timestamp).await {
        return Ok(Some(cached_frame));
    }
    
    // 3. Cache miss: Convert 5-minute window from MP4 segments
    let cache_window_start = timestamp - Duration::minutes(2) - Duration::seconds(30);
    let cache_window_end = timestamp + Duration::minutes(2) + Duration::seconds(30);
    
    self.convert_and_cache_mp4_window(camera_id, cache_window_start, cache_window_end).await?;
    
    // 4. Return frame from newly populated cache
    self.get_cached_mp4_frame(camera_id, window_id, timestamp).await.map(Some).or(Ok(None))
}
```

### 2. Unified Frame Cache Architecture

**Cache Structure**:
```rust
struct UnifiedFrameCache {
    // Live recording buffer (reuse existing frame_buffer from video_segmenter_loop)
    live_buffers: HashMap<String, LiveRecordingBuffer>,
    
    // MP4 conversion cache for historical data
    mp4_cache: HashMap<String, HashMap<i64, CacheWindow>>, // camera_id -> windows
}

struct LiveRecordingBuffer {
    frames: VecDeque<TimestampedFrame>,  // Rolling buffer of recent frames
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>,
    max_frames: usize,  // ~4500 frames for 5 minutes at 15fps
}

struct TimestampedFrame {
    timestamp: DateTime<Utc>,
    frame_data: Bytes,  // Reuse the same Bytes from broadcast
}

struct CacheWindow {
    start_time: DateTime<Utc>,
    end_time: DateTime<Utc>, 
    frames: BTreeMap<i64, RecordedFrame>, // i64 = timestamp_millis for fast lookup
    last_accessed: Instant,
    source: CacheSource,  // Track whether from live buffer or MP4 conversion
}

enum CacheSource {
    LiveRecording,   // Frames from active recording buffer
    Mp4Conversion,   // Frames converted from MP4 segments
}
```

**Integration with Existing Recording Buffer**:
```rust
// In video_segmenter_loop, also maintain live buffer cache
async fn video_segmenter_loop(...) {
    let mut frame_buffer = Vec::new();  // Existing buffer for MP4 creation
    let live_cache = unified_cache.get_live_buffer(&camera_id);
    
    loop {
        match frame_receiver.recv().await {
            Ok(frame_data) => {
                // Add to MP4 segment buffer (existing)
                frame_buffer.push(frame_data.clone());
                
                // ALSO add to live cache for instant playback
                live_cache.add_frame(TimestampedFrame {
                    timestamp: Utc::now(),
                    frame_data: frame_data.clone(),
                }).await;
                
                // Continue with existing MP4 segment creation logic...
            }
        }
    }
}
```

**5-Minute Window Strategy**:
- Live buffer maintains most recent 5 minutes of frames
- MP4 cache windows aligned to 5-minute boundaries
- Seamless transition from live buffer to MP4 cache as time progresses

### 3. Playback Flow (StartReplay)

**Initial Load**:
```rust
// StartReplay { from: 10:05:00, to: 11:30:00 }
1. Calculate window containing 'from': 10:02:30 - 10:07:30
2. Convert MP4 segments to frames for this window
3. Start streaming from cache
4. Cache State: [Window A: 10:02:30-10:07:30]
```

**Rolling Cache During Playback**:
```rust
// At playback timestamp 10:07:00 (30 seconds before window end)
1. Start background preloading: Window B (10:07:30-10:12:30)
2. Continue streaming from Window A
3. Cache State: [Window A, Window B]

// At playback timestamp 10:08:00 (moved to Window B)  
4. Stream from Window B
5. Remove old Window A (LRU cleanup)
6. Cache State: [Window B]

// Predictive loading continues...
7. At 10:12:00, preload Window C (10:12:30-10:17:30)
8. Cache State: [Window B, Window C]
```

**Memory Management**:
- Keep 2-3 windows per camera (15-20 minutes of frames)
- ~50-100MB memory per 5-minute window
- LRU eviction based on `last_accessed` time
- Background cleanup removes oldest windows

### 4. Goto Functionality

**Live Buffer Hit (Ultra-Fast)**:
```rust
// goto: [timestamp within last 5 minutes of recording]
1. Check live recording buffer first
2. Binary search in VecDeque for closest frame
3. Return frame directly from memory
4. Response time: ~0.05ms
```

**Cache Hit (Instant)**:
```rust
// goto: 10:09:15 (Window B is cached from MP4)
1. Live buffer check: miss (too old)
2. Calculate window_id for 10:09:15 → Window B
3. Lookup in MP4 cache: BTreeMap range query
4. Return closest frame ≤ timestamp
5. Response time: ~0.1ms
```

**Cache Miss (5-Minute Conversion)**:
```rust
// goto: 11:15:00 (no cached window)
1. Live buffer check: miss
2. MP4 cache check: miss
3. Calculate window: 11:12:30-11:17:30
4. Find overlapping MP4 segments (query recording_mp4 table only)
5. Convert segments to frames using FFmpeg
6. Cache frames in new window
7. Return requested frame
8. Response time: ~2-5 seconds (one-time cost)
```

**Important**: The `recording_mjpeg` table is **NEVER** queried during goto operations.

### 5. FFmpeg Integration

**Frame Extraction Command**:
```bash
ffmpeg -ss {start_offset} -i {segment.mp4} -t 300 -r 15 -f mjpeg -frame_pts true pipe:1
```

**Parameters**:
- `start_offset`: Seconds from MP4 segment start to window start
- `t 300`: Extract 5 minutes (300 seconds) of frames
- `r 15`: Extract at 15fps (configurable)
- `frame_pts true`: Preserve original timestamps

**Conversion Process**:
```rust
async fn convert_and_cache_mp4_window(&self, camera_id: &str, start: DateTime<Utc>, end: DateTime<Utc>) -> Result<()> {
    // 1. Find MP4 segments that overlap with the 5-minute window
    let segments = self.list_video_segments(camera_id, start, end).await?;
    
    for segment in segments {
        // 2. Calculate exact time offsets within the MP4 file
        let segment_start_offset = start.max(segment.start_time)
            .signed_duration_since(segment.start_time)
            .num_seconds() as f64;
        let extraction_duration = (end.min(segment.end_time) - start.max(segment.start_time))
            .num_seconds() as f64;
            
        // 3. Extract frames from MP4 segment for this time window
        let frames = extract_frames_from_mp4_range(&segment, segment_start_offset, extraction_duration).await?;
        
        // 4. Store in cache with timestamp-based indexing
        let window_id = calculate_5min_window_id(start);
        self.cache_frames(camera_id, window_id, start, end, frames).await;
    }
    
    Ok(())
}
```

## Implementation Plan

### Phase 1: Core Infrastructure
- [ ] Implement `UnifiedFrameCache` with live buffer and MP4 cache
- [ ] Integrate live buffer with existing `video_segmenter_loop`
- [ ] Create FFmpeg frame extraction utilities
- [ ] Remove all `recording_mjpeg` table queries from playback code
- [ ] Implement window ID calculation and alignment

### Phase 2: Live Buffer Integration
- [ ] Modify `video_segmenter_loop` to maintain `LiveRecordingBuffer`
- [ ] Implement VecDeque-based rolling buffer (5-minute capacity)
- [ ] Add timestamp-based binary search for frame lookup
- [ ] Ensure zero-copy frame sharing between buffer and MP4 creation

### Phase 3: Streaming Integration
- [ ] Create `CachedFrameStream` implementation (no DB access)
- [ ] Add seamless transition from live buffer to MP4 cache
- [ ] Implement predictive cache preloading during playback
- [ ] Extend `create_replay_stream()` to use unified cache

### Phase 4: Performance Optimization
- [ ] Add concurrent FFmpeg processing for multiple segments
- [ ] Implement memory usage monitoring and limits
- [ ] Add cache hit/miss metrics (live vs MP4 vs conversion)
- [ ] Optimize frame indexing and lookup performance

### Phase 5: Configuration & Management
- [ ] Add configuration for live buffer size and retention
- [ ] Implement cache warmup strategies
- [ ] Add admin APIs for cache management
- [ ] Create cleanup tasks for cache lifecycle

## Configuration Options

```json
{
  "mp4_segment_streaming": {
    "enabled": true,
    "unified_cache": {
      "live_buffer": {
        "enabled": true,
        "duration_minutes": 5,
        "max_frames_per_camera": 4500,  // 5 min at 15fps
        "memory_mb_per_camera": 75
      },
      "mp4_cache": {
        "max_memory_mb": 512,
        "window_duration_minutes": 5,
        "max_windows_per_camera": 3,
        "cleanup_interval_minutes": 10,
        "preload_threshold_seconds": 30
      }
    },
    "conversion": {
      "ffmpeg_fps": 15,
      "concurrent_processes": 2,
      "timeout_seconds": 30
    },
    "database_playback": {
      "enabled": false  // Set to true to fallback to old DB-based playback
    }
  }
}
```

## Performance Characteristics

| Operation | Old (DB Query) | Live Buffer | Cached MP4 | Uncached MP4 |
|-----------|---------------|-------------|------------|--------------|
| **goto** | ~1-5ms | ~0.05ms | ~0.1ms | ~2-5s |
| **playback start** | ~1-5ms | ~0.05ms | ~0.1ms | ~2-5s |
| **streaming fps** | 15-30fps* | Native | Native | Native** |
| **memory per camera** | ~10MB | ~75MB | ~150MB | ~150MB |
| **DB queries per second** | 15-30 | **0** | **0** | **0** |

*Limited by database query speed
**After initial cache population

### Key Performance Improvements:
- **Zero database queries** during playback (vs 15-30 queries/second before)
- **50% faster** access to live recording frames (0.05ms vs 1ms)
- **No I/O blocking** - pure memory access for all cached frames
- **Reduced database load** - `recording_mjpeg` table only written, never read during playback

## Error Handling & Edge Cases

### Network/FFmpeg Failures
- Retry failed conversions with exponential backoff
- Fallback to partial cache windows if some segments fail
- Return empty frames for completely failed windows

### Memory Pressure
- Implement cache size limits and forced eviction
- Monitor system memory usage
- Graceful degradation with smaller cache windows

### Concurrent Access
- Thread-safe cache operations with RwLock
- Handle multiple clients requesting same time windows
- Prevent duplicate FFmpeg processes for same segments

### Timestamp Edge Cases  
- Handle requests at exact window boundaries
- Manage overlapping MP4 segments correctly
- Ensure frame timestamp accuracy during conversion

## Testing Strategy

### Unit Tests
- Cache window calculation and alignment
- Frame lookup and indexing logic
- FFmpeg command generation and parsing
- LRU eviction and memory management

### Integration Tests
- End-to-end playback with MP4 fallback
- Goto functionality across cached/uncached windows
- Multiple concurrent client streaming
- Cache persistence across server restarts

### Performance Tests
- Streaming latency and throughput measurement
- Memory usage profiling under load
- Cache hit/miss ratio optimization
- FFmpeg conversion performance benchmarks

## Future Enhancements

### Advanced Caching
- **Smart Preloading**: Machine learning-based prediction of user access patterns
- **Compression**: Optional frame compression in cache to reduce memory usage
- **Distributed Cache**: Multi-node cache sharing for horizontal scaling

### Streaming Optimizations  
- **Variable FPS**: Adaptive frame rate based on available bandwidth
- **Quality Scaling**: Multiple resolution tiers in cache
- **Progressive Loading**: Partial frame loading for faster initial response

### User Experience
- **Loading Indicators**: Real-time conversion progress for UI
- **Scrubbing Support**: Optimized random access for timeline scrubbing
- **Bandwidth Adaptation**: Quality adjustment based on connection speed

---

*This design provides instant goto functionality and smooth streaming playback while efficiently managing memory usage through a rolling cache strategy.*