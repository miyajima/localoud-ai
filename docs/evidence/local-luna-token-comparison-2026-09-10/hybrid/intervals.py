from typing import Any, List, Optional, Tuple, Union


Interval = Tuple[int, int]
Intervals = Union[List[Interval], Tuple[Interval, ...]]

__all__ = ['normalize', 'subtract', 'earliest_slot']


def _is_integer(value: Any) -> bool:
    return isinstance(value, int) and not isinstance(value, bool)


def _validate_interval(interval: Any) -> Interval:
    if not isinstance(interval, (list, tuple)):
        raise ValueError('Each interval must be a list or tuple')
    if len(interval) != 2:
        raise ValueError('Each interval must have exactly two elements')

    start, end = interval
    if not _is_integer(start) or not _is_integer(end):
        raise ValueError('Interval endpoints must be integers; bool is not allowed')
    if start >= end:
        raise ValueError('Interval start must be less than end')

    return (start, end)


def _validate_intervals(intervals: Any) -> List[Interval]:
    if not isinstance(intervals, (list, tuple)):
        raise ValueError('Outer collection must be a list or tuple')
    return [_validate_interval(interval) for interval in intervals]


def _validate_duration(duration: Any) -> int:
    if not _is_integer(duration) or duration <= 0:
        raise ValueError('Duration must be a positive integer; bool is not allowed')
    return duration


def _normalize_validated(intervals: List[Interval]) -> List[Interval]:
    if not intervals:
        return []

    ordered = sorted(intervals, key=lambda interval: (interval[0], interval[1]))
    merged: List[Interval] = []
    current_start, current_end = ordered[0]

    for start, end in ordered[1:]:
        if start <= current_end:
            if end > current_end:
                current_end = end
        else:
            merged.append((current_start, current_end))
            current_start, current_end = start, end

    merged.append((current_start, current_end))
    return merged


def normalize(intervals: Intervals) -> List[Tuple[int, int]]:
    return _normalize_validated(_validate_intervals(intervals))


def _subtract_normalized(
    intervals: List[Interval], exclusions: List[Interval]
) -> List[Interval]:
    result: List[Interval] = []

    for interval_start, interval_end in intervals:
        cursor = interval_start

        for exclusion_start, exclusion_end in exclusions:
            if exclusion_end <= cursor:
                continue
            if exclusion_start >= interval_end:
                break

            if exclusion_start > cursor:
                result.append((cursor, exclusion_start))

            if exclusion_end > cursor:
                cursor = exclusion_end
            if cursor >= interval_end:
                break

        if cursor < interval_end:
            result.append((cursor, interval_end))

    return result


def subtract(intervals: Intervals, exclusions: Intervals) -> List[Tuple[int, int]]:
    interval_values = _validate_intervals(intervals)
    exclusion_values = _validate_intervals(exclusions)

    normalized_intervals = _normalize_validated(interval_values)
    normalized_exclusions = _normalize_validated(exclusion_values)
    return _subtract_normalized(normalized_intervals, normalized_exclusions)


def earliest_slot(
    busy: Intervals, duration: Any, window: Interval
) -> Optional[int]:
    busy_values = _validate_intervals(busy)
    duration_value = _validate_duration(duration)
    window_value = _validate_interval(window)

    normalized_busy = _normalize_validated(busy_values)
    free_intervals = _subtract_normalized([window_value], normalized_busy)

    for start, end in free_intervals:
        if end - start >= duration_value:
            return start
    return None
