"""Operations on half-open interval sets without modifying caller inputs."""


def normalize(intervals):
    """Return sorted, nonempty intervals with overlaps and touches merged."""
    ordered = []
    for start, end in intervals:
        if start > end:
            raise ValueError("interval start must not exceed its end")
        if start != end:
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
    removals = normalize(cuts)
    result = []
    cut_index = 0
    for start, end in sources:
        cursor = start
        while cut_index < len(removals) and removals[cut_index][1] <= cursor:
            cut_index += 1
        index = cut_index
        while index < len(removals) and removals[index][0] < end:
            cut_start, cut_end = removals[index]
            if cut_start > cursor:
                result.append((cursor, cut_start))
            cursor = max(cursor, cut_end)
            if cursor >= end:
                break
            index += 1
        cut_index = index
        if cursor < end:
            result.append((cursor, end))
    return result


def contains(intervals, point):
    """Return whether point belongs to the interval set, validating all bounds."""
    return any(start <= point < end for start, end in normalize(intervals))
