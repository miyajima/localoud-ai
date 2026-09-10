
import json
from typing import Any, List, Tuple, Union


Interval = Tuple[int, int]
Intervals = Union[List[Interval], Tuple[Interval, ...]]


def _validate_interval(interval: Any) -> Interval:
    if not isinstance(interval, (list, tuple)):
        raise ValueError("Intervals must be lists or tuples")
    if len(interval) != 2:
        raise ValueError("Each interval must have exactly two elements")
    start, end = interval
    if not isinstance(start, int) or isinstance(start, bool):
        raise ValueError("Interval start must be an integer (bool is not allowed)")
    if not isinstance(end, int) or isinstance(end, bool):
        raise ValueError("Interval end must be an integer (bool is not allowed)")
    if start >= end:
        raise ValueError("Interval start must be less than end")
    return (start, end)


def _validate_intervals(intervals: Intervals) -> List[Interval]:
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("Outer collection must be a list or tuple")
    normalized = []
    for item in intervals:
        normalized.append(_validate_interval(item))
    return normalized


def normalize(intervals: Intervals) -> List[Tuple[int, int]]:
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("Input must be a list or tuple")
    if intervals is None:
        raise ValueError("Input cannot be None")
    intervals = _validate_intervals(intervals)
    if not intervals:
        return []
    intervals.sort(key=lambda x: (x[0], x[1]))
    merged = []
    current_start, current_end = intervals[0]
    for start, end in intervals[1:]:
        if start <= current_end:
            current_end = max(current_end, end)
        else:
            merged.append((current_start, current_end))
            current_start, current_end = start, end
    merged.append((current_start, current_end))
    return merged


def _validate_exclusions(exclusions: Intervals) -> List[Interval]:
    if not isinstance(exclusions, (list, tuple)):
        raise ValueError("Exclusions must be a list or tuple")
    if exclusions is None:
        raise ValueError("Exclusions cannot be None")
    return _validate_intervals(exclusions)


def subtract(intervals: Intervals, exclusions: Intervals) -> List[Tuple[int, int]]:
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("intervals must be a list or tuple")
    if intervals is None:
        raise ValueError("intervals cannot be None")
    if not isinstance(exclusions, (list, tuple)):
        raise ValueError("exclusions must be a list or tuple")
    if exclusions is None:
        raise ValueError("exclusions cannot be None")
    intervals = _validate_intervals(intervals)
    exclusions = _validate_exclusions(exclusions)
    if not intervals:
        return []
    if not exclusions:
        return normalize(intervals)
    excl = normalize(exclusions)
    result = []
    current_start, current_end = intervals[0]
    for ex_start, ex_end in excl:
        if ex_end <= current_start:
            current_start = max(current_start, ex_start)
            continue
        if ex_start >= current_end:
            result.append((current_start, current_end))
            current_start, current_end = ex_start, ex_end
        else:
            if current_start < ex_start:
                result.append((current_start, ex_start))
            current_start = max(current_end, ex_end)
    if current_start < current_end:
        result.append((current_start, current_end))
    return result


def _validate_busy(busy: Intervals) -> List[Interval]:
    if not isinstance(busy, (list, tuple)):
        raise ValueError("busy must be a list or tuple")
    if busy is None:
        raise ValueError("busy cannot be None")
    return _validate_intervals(busy)


def _validate_window(window: Intervals) -> Interval:
    if not isinstance(window, (list, tuple)):
        raise ValueError("window must be a list or tuple")
    if window is None:
        raise ValueError("window cannot be None")
    return _validate_interval(window)


def _validate_duration(duration: Any) -> int:
    if not isinstance(duration, int) or isinstance(duration, bool):
        raise ValueError("duration must be a positive integer (bool is not allowed)")
    if duration <= 0:
        raise ValueError("duration must be positive")
    return duration


def earliest_slot(busy: Intervals, duration: Any, window: Intervals) -> Any:
    if not isinstance(busy, (list, tuple)):
        raise ValueError("busy must be a list or tuple")
    if busy is None:
        raise ValueError("busy cannot be None")
    if not isinstance(duration, int) or isinstance(duration, bool):
        raise ValueError("duration must be a positive integer (bool is not allowed)")
    if duration <= 0:
        raise ValueError("duration must be positive")
    if not isinstance(window, (list, tuple)):
        raise ValueError("window must be a list or tuple")
    if window is None:
        raise ValueError("window cannot be None")
    busy = _validate_busy(busy)
    window = _validate_window(window)
    duration = _validate_duration(duration)
    busy = normalize(busy)
    window_start, window_end = window
    if window_start >= window_end:
        raise ValueError("window must satisfy start < end")
    if duration > window_end - window_start:
        return None
    busy = normalize(busy)
    busy = [b for b in busy if b[0] < window_end]
    busy.sort(key=lambda x: (x[0], x[1]))
    current = window_start
    for start, end in busy:
        if start >= current:
            if current + duration <= end:
                return current
            current = max(current, end)
        else:
            if current + duration <= start:
                return current
            current = max(current, end)
    if current + duration <= window_end:
        return current
    return None


if __name__ == "__main__":
    # Quick self-test
    print(normalize([(3, 5), (1, 3)]))
    print(subtract([(0, 10)], [(2, 4), (6, 20)]))
    print(earliest_slot([(1, 3)], 2, (0, 5)))
    print(json.dumps({"normalize": normalize([(3, 5), (1, 3)]),
                         "subtract": subtract([(0, 10)], [(2, 4), (6, 20)]),
                         "earliest_slot": earliest_slot([(1, 3)], 2, (0, 5))}))
