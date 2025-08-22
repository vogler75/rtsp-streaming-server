-- MP4 Hourly Storage Statistics
-- Shows MP4 segment storage grouped by hour for the last 24 hours

SELECT 
    rs.camera_id,
    strftime('%Y-%m-%d %H:00:00', rm.start_time) as hour,
    COUNT(*) as segment_count,
    ROUND(SUM(rm.size_bytes) / 1024.0 / 1024.0, 2) as size_mb,
    ROUND(AVG(rm.size_bytes) / 1024.0 / 1024.0, 2) as avg_segment_mb,
    ROUND(SUM((julianday(rm.end_time) - julianday(rm.start_time)) * 1440), 1) as total_minutes,
    ROUND(AVG((julianday(rm.end_time) - julianday(rm.start_time)) * 1440), 1) as avg_segment_minutes,
    ROUND(SUM(rm.size_bytes) / SUM((julianday(rm.end_time) - julianday(rm.start_time)) * 86400), 0) as bytes_per_second,
    MIN(rm.start_time) as first_segment,
    MAX(rm.end_time) as last_segment
FROM recording_mp4 rm
JOIN recording_sessions rs ON rm.session_id = rs.id
WHERE rm.start_time >= datetime('now', '-24 hours')
GROUP BY rs.camera_id, strftime('%Y-%m-%d %H:00:00', rm.start_time)
ORDER BY rs.camera_id, hour DESC;