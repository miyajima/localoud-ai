def normalize(intervals):
    """
    Return sorted disjoint union of intervals.
    Merge overlapping AND touching intervals.
    Empty input yields [].
    """
    if not intervals:
        return []
    
    # Validate all inputs
    for i, interval in enumerate(intervals):
        if not isinstance(interval, (list, tuple)):
            raise ValueError(f"Interval {i} is not a list or tuple")
        if len(interval) != 2:
            raise ValueError(f"Interval {i} must have exactly 2 elements")
        start, end = interval
        if not isinstance(start, int) or not isinstance(end, int):
            raise ValueError(f"Interval {i} coordinates must be integers")
        if start >= end:
            raise ValueError(f"Interval {i} must have start < end")
    
    # Sort by start time
    sorted_intervals = sorted(intervals, key=lambda x: x[0])
    
    merged = []
    current_start, current_end = sorted_intervals[0]
    
    for next_start, next_end in sorted_intervals[1:]:
        # Merge if overlapping or touching
        if next_start <= current_end:
            current_end = max(current_end, next_end)
        else:
            merged.append((current_start, current_end))
            current_start, current_end = next_start, next_end
    
    merged.append((current_start, current_end))
    return merged


def subtract(intervals, exclusions):
    """
    Return normalize(intervals) minus the union of exclusions.
    Sorted nonempty disjoint intervals.
    Touching exclusion boundaries remove no extra points.
    Empty input yields [] after full validation.
    """
    # Validate intervals
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("intervals must be a list or tuple")
    for i, interval in enumerate(intervals):
        if not isinstance(interval, (list, tuple)):
            raise ValueError(f"Interval {i} is not a list or tuple")
        if len(interval) != 2:
            raise ValueError(f"Interval {i} must have exactly 2 elements")
        start, end = interval
        if not isinstance(start, int) or not isinstance(end, int):
            raise ValueError(f"Interval {i} coordinates must be integers")
        if start >= end:
            raise ValueError(f"Interval {i} must have start < end")
    
    # Validate exclusions
    if not isinstance(exclusions, (list, tuple)):
        raise ValueError("exclusions must be a list or tuple")
    for i, interval in enumerate(exclusions):
        if not isinstance(interval, (list, tuple)):
            raise ValueError(f"Exclusion {i} is not a list or tuple")
        if len(interval) != 2:
            raise ValueError(f"Exclusion {i} must have exactly 2 elements")
        start, end = interval
        if not isinstance(start, int) or not isinstance(end, int):
            raise ValueError(f"Exclusion {i} coordinates must be integers")
        if start >= end:
            raise ValueError(f"Exclusion {i} must have start < end")
    
    # Normalize intervals
    normalized_intervals = normalize(intervals)
    
    # Normalize exclusions
    normalized_exclusions = normalize(exclusions)
    
    if not normalized_intervals:
        return []
    
    # Subtract exclusions from intervals
    result = []
    interval_idx = 0
    exclusion_idx = 0
    
    while interval_idx < len(normalized_intervals) and exclusion_idx < len(normalized_exclusions):
        interval_start, interval_end = normalized_intervals[interval_idx]
        exclusion_start, exclusion_end = normalized_exclusions[exclusion_idx]
        
        # If interval is completely before exclusion
        if interval_end <= exclusion_start:
            result.append((interval_start, interval_end))
            interval_idx += 1
        # If interval is completely after exclusion
        elif interval_start >= exclusion_end:
            exclusion_idx += 1
        # Overlapping cases
        else:
            # Interval starts before exclusion ends
            if interval_start < exclusion_start:
                result.append((interval_start, exclusion_start))
            # Interval ends after exclusion starts
            if interval_end > exclusion_end:
                result.append((exclusion_end, interval_end))
            interval_idx += 1
            exclusion_idx += 1
    
    # Add remaining intervals
    while interval_idx < len(normalized_intervals):
        interval_start, interval_end = normalized_intervals[interval_idx]
        result.append((interval_start, interval_end))
        interval_idx += 1
    
    # Filter out empty intervals (should not happen with correct logic, but safety check)
    result = [(s, e) for s, e in result if s < e]
    
    return result


def earliest_slot(busy, duration, window):
    """
    Return the earliest integer start t such that [t,t+duration) lies inside window and does not overlap busy.
    Busy entries may extend outside window.
    Return None if no fit.
    Validate all arguments even if no fit is possible.
    """
    # Validate busy
    if not isinstance(busy, (list, tuple)):
        raise ValueError("busy must be a list or tuple")
    for i, interval in enumerate(busy):
        if not isinstance(interval, (list, tuple)):
            raise ValueError(f"Busy interval {i} is not a list or tuple")
        if len(interval) != 2:
            raise ValueError(f"Busy interval {i} must have exactly 2 elements")
        start, end = interval
        if not isinstance(start, int) or not isinstance(end, int):
            raise ValueError(f"Busy interval {i} coordinates must be integers")
        if start >= end:
            raise ValueError(f"Busy interval {i} must have start < end")
    
    # Validate window
    if not isinstance(window, (list, tuple)):
        raise ValueError("window must be a list or tuple")
    if len(window) != 2:
        raise ValueError("window must have exactly 2 elements")
    start, end = window
    if not isinstance(start, int) or not isinstance(end, int):
        raise ValueError("window coordinates must be integers")
    if start >= end:
        raise ValueError("window must have start < end")
    
    # Validate duration
    if isinstance(duration, bool):
        raise ValueError("duration must be an integer, not a boolean")
    if not isinstance(duration, int):
        raise ValueError("duration must be an integer")
    if duration <= 0:
        raise ValueError("duration must be positive")
    
    # Normalize busy intervals
    normalized_busy = normalize(busy)
    
    # Normalize window
    normalized_window = normalize([window])
    
    # Filter busy intervals that are outside the window
    # We only care about busy intervals that overlap with the window
    # Actually, the problem says "Busy entries may extend outside window" and we need [t, t+duration) to lie INSIDE window
    # So we only need to consider busy intervals that overlap with the window
    
    # Create a set of busy points within the window
    # Since coordinates can be large, we can't use a set of all points
    # We need to check overlap with each busy interval
    
    # The slot [t, t+duration) must be inside [window_start, window_end)
    # So t >= window_start and t+duration <= window_end
    # => t >= window_start and t <= window_end - duration
    
    min_t = window[0]
    max_t = window[1] - duration
    
    if min_t > max_t:
        return None
    
    # Find the first t in [min_t, max_t] that doesn't overlap any busy interval
    # We can iterate through busy intervals that overlap with the window
    
    # Collect all busy intervals that overlap with the window
    # An interval [b_start, b_end) overlaps with [w_start, w_end) if max(b_start, w_start) < min(b_end, w_end)
    # But we only care about the part of busy that is within [min_t, max_t]
    
    # Let's create a list of busy intervals clipped to the window
    # Actually, we just need to know which t values are blocked
    
    # Since we need the earliest t, we can iterate t from min_t upwards
    # But t could be large, so we need a smarter approach
    
    # The blocked regions are the union of busy intervals intersected with [min_t, max_t]
    # We can compute this union and then find the first gap
    
    blocked_intervals = []
    for b_start, b_end in normalized_busy:
        # Clip busy interval to window
        # Intersection of [b_start, b_end) and [window_start, window_end)
        inter_start = max(b_start, window[0])
        inter_end = min(b_end, window[1])
        
        if inter_start < inter_end:
            # Clip to [min_t, max_t]
            clip_start = max(inter_start, min_t)
            clip_end = min(inter_end, max_t)
            
            if clip_start < clip_end:
                blocked_intervals.append((clip_start, clip_end))
    
    if not blocked_intervals:
        return min_t
    
    # Normalize blocked intervals
    blocked_intervals = normalize(blocked_intervals)
    
    # Find the first gap in [min_t, max_t]
    # We need to find the smallest t in [min_t, max_t] such that [t, t+duration) doesn't overlap any blocked interval
    
    # The blocked intervals partition [min_t, max_t] into free segments
    # We can iterate through the blocked intervals and find gaps
    
    current_t = min_t
    
    for b_start, b_end in blocked_intervals:
        # If the current t is before this blocked interval
        if current_t < b_start:
            # Check if we can fit the duration before this blocked interval
            if current_t + duration <= b_start:
                return current_t
            else:
                # We need to skip past this blocked interval
                current_t = b_end
        else:
            # current_t is inside or after the blocked interval
            # We need to skip past this blocked interval
            current_t = b_end
    
    # If we've gone through all blocked intervals and still haven't found a slot
    # Check if there's space after the last blocked interval
    if current_t + duration <= max_t:
        return current_t
    
    return None