import sys

def normalize(intervals):
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("intervals must be a list or tuple")
    if len(intervals) != 0:
        for iv in intervals:
            if not (isinstance(iv, (list, tuple)) and len(iv) == 2):
                raise ValueError("each interval must be a list or tuple of two integers")
            if not (isinstance(iv[0], int) and isinstance(iv[1], int)):
                raise ValueError("interval bounds must be integers")
            if iv[0] >= iv[1]:
                raise ValueError("interval start must be less than end")
    sorted_intervals = sorted(intervals, key=lambda x: x[0])
    merged = []
    for iv in sorted_intervals:
        if not merged:
            merged.append(iv)
        else:
            last = merged[-1]
            if iv[0] <= last[1]:
                merged[-1] = (last[0], max(last[1], iv[1]))
            else:
                merged.append(iv)
    return merged

def subtract(intervals, exclusions):
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("intervals must be a list or tuple")
    if not isinstance(exclusions, (list, tuple)):
        raise ValueError("exclusions must be a list or tuple")
    if len(intervals) == 0:
        if len(exclusions) == 0:
            return []
        else:
            raise ValueError("exclusions cannot be non-empty when intervals is empty")
    if len(exclusions) == 0:
        return normalize(intervals)
    norm_intervals = normalize(intervals)
    norm_exclusions = normalize(exclusions)
    result = []
    for iv in norm_intervals:
        for ex in norm_exclusions:
            if iv[0] < ex[1] and iv[1] > ex[0]:
                if iv[0] < ex[0]:
                    result.append((iv[0], ex[0]))
                if ex[1] < iv[1]:
                    result.append((ex[1], iv[1]))
    return normalize(result)

def earliest_slot(busy, duration, window):
    if not isinstance(busy, (list, tuple)):
        raise ValueError("busy must be a list or tuple")
    if not isinstance(duration, int) or isinstance(duration, bool):
        raise ValueError("duration must be an integer")
    if duration <= 0:
        raise ValueError("duration must be positive")
    if not isinstance(window, (list, tuple)):
        raise ValueError("window must be a list or tuple")
    if len(window) != 2:
        raise ValueError("window must be a list or tuple of two integers")
    if not (isinstance(window[0], int) and isinstance(window[1], int)):
        raise ValueError("window bounds must be integers")
    if window[0] >= window[1]:
        raise ValueError("window start must be less than end")
    if not isinstance(busy, (list, tuple)):
        raise ValueError("busy must be a list or tuple")
    if len(busy) == 0:
        norm_busy = []
    else:
        norm_busy = normalize(busy)
    if not norm_busy:
        if window[0] + duration <= window[1]:
            return window[0]
        else:
            return None
    for t in range(window[0], window[1] - duration + 1):
        slot = (t, t + duration)
        overlap = False
        for b in norm_busy:
            if not (slot[1] <= b[0] or slot[0] >= b[1]):
                overlap = True
                break
        if not overlap:
            return t
    return None
