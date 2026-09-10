__all__ = ['normalize', 'subtract', 'earliest_slot']


def _validate_interval(interval):
    if not isinstance(interval, (list, tuple)) or len(interval) != 2:
        raise ValueError('invalid interval')

    start, end = interval
    if (
        isinstance(start, bool)
        or not isinstance(start, int)
        or isinstance(end, bool)
        or not isinstance(end, int)
        or start >= end
    ):
        raise ValueError('invalid interval')

    return (start, end)


def _validate_collection(intervals):
    if not isinstance(intervals, (list, tuple)):
        raise ValueError('invalid interval collection')
    return [_validate_interval(interval) for interval in intervals]


def _normalize_validated(intervals):
    merged = []

    for start, end in sorted(intervals):
        if not merged or start > merged[-1][1]:
            merged.append((start, end))
        elif end > merged[-1][1]:
            previous_start, previous_end = merged[-1]
            merged[-1] = (previous_start, end)

    return merged


def normalize(intervals):
    return _normalize_validated(_validate_collection(intervals))


def subtract(intervals, exclusions):
    base = _validate_collection(intervals)
    excluded = _validate_collection(exclusions)

    base = _normalize_validated(base)
    excluded = _normalize_validated(excluded)

    result = []
    for start, end in base:
        cursor = start

        for exclusion_start, exclusion_end in excluded:
            if exclusion_end <= cursor:
                continue
            if exclusion_start >= end:
                break

            if exclusion_start > cursor:
                result.append((cursor, exclusion_start))

            if exclusion_end > cursor:
                cursor = exclusion_end
            if cursor >= end:
                break

        if cursor < end:
            result.append((cursor, end))

    return result


def _validate_duration(duration):
    if (
        isinstance(duration, bool)
        or not isinstance(duration, int)
        or duration <= 0
    ):
        raise ValueError('invalid duration')


def earliest_slot(busy, duration, window):
    busy = _validate_collection(busy)
    _validate_duration(duration)
    window_start, window_end = _validate_interval(window)
    busy = _normalize_validated(busy)

    candidate = window_start

    for busy_start, busy_end in busy:
        if busy_end <= candidate:
            continue
        if busy_start >= window_end:
            break

        if busy_start > candidate and busy_start - candidate >= duration:
            return candidate

        if busy_end > candidate:
            candidate = busy_end
        if candidate >= window_end:
            return None

    if candidate + duration <= window_end:
        return candidate
    return None