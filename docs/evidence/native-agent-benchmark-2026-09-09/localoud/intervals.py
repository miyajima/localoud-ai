"""Operations on half-open interval sets, without mutating inputs."""


def normalize(intervals):
    """Return sorted nonempty intervals, merging overlaps and touching bounds.

    Reversed bounds raise ValueError, including in otherwise empty sets.
    """
    ordered = []
    for start, end in intervals:
        if start > end:
            raise ValueError("interval start must not exceed end")
        if start < end:
            ordered.append((start, end))
    ordered.sort()
    merged = []
    for start, end in ordered:
        if merged and start <= merged[-1][1]:
            merged[-1] = (merged[-1][0], max(merged[-1][1], end))
        else:
            merged.append((start, end))
    return merged


def subtract(intervals, cuts):
    """Return the normalized set difference of intervals and cuts."""
    sources = normalize(intervals)
    cuts = normalize(cuts)
    result = []
    index = 0
    for start, end in sources:
        cursor = start
        while index < len(cuts) and cuts[index][1] <= cursor:
            index += 1
        while index < len(cuts) and cuts[index][0] < end:
            cut_start, cut_end = cuts[index]
            if cursor < cut_start:
                result.append((cursor, cut_start))
            cursor = max(cursor, cut_end)
            if cursor >= end:
                # Retain this cut: it may also cover a later source interval.
                break
            index += 1
        if cursor < end:
            result.append((cursor, end))
    return result


def contains(intervals, point):
    """Return whether point belongs to the validated half-open interval set."""
    return any(start <= point < end for start, end in normalize(intervals))
