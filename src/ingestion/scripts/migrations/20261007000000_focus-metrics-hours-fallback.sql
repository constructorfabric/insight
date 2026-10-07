-- Rows stored before the eight-hour fallback took effect carry a zero-hour day.
-- INVARIANT: the model never writes 0 any more, so the guard makes a re-run a no-op.

ALTER TABLE silver.class_focus_metrics
    UPDATE working_hours_per_day = 8.0,
           focus_time_pct        = round(greatest(toFloat64(0), 100.0 - (meeting_hours / 8.0) * 100.0), 2),
           dev_time_h            = round(greatest(toFloat64(0), 8.0 - meeting_hours), 4)
    WHERE working_hours_per_day = 0
    SETTINGS mutations_sync = 1;
