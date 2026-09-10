def _validate_interval(x):
    if isinstance(x, bool):
        raise ValueError("interval must be a list or tuple of two integers")
    if not isinstance(x, (list, tuple)):
        raise ValueError("interval must be a list or tuple of two integers")
    if len(x) != 2:
        raise ValueError("interval must have exactly two elements")
    a, b = x
    if isinstance(a, bool) or isinstance(b, bool):
        raise ValueError("interval coordinates must be integers, not booleans")
    if not isinstance(a, int) or not isinstance(b, int):
        raise ValueError("interval coordinates must be integers")
    if not a < b:
        raise ValueError("interval start must be less than end")
    return (a, b)

def _validate_collection(x, name):
    if isinstance(x, bool):
        raise ValueError(name + " must be a list or tuple")
    if not isinstance(x, (list, tuple)):
        raise ValueError(name + " must be a list or tuple")
    return [_validate_interval(i) for i in x]

def normalize(intervals):
    if isinstance(intervals, bool):
        raise ValueError("intervals must be a list or tuple")
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("intervals must be a list or tuple")
    ivs = [_validate_interval(i) for i in intervals]
    if not ivs:
        return []
    ivs.sort()
    res = [ivs[0]]
    for s, e in ivs[1:]:
        if s <= res[-1][1]:
            if e > res[-1][1]:
                res[-1] = (res[-1][0], e)
        else:
            res.append((s, e))
    return res

def subtract(intervals, exclusions):
    if isinstance(intervals, bool):
        raise ValueError("intervals must be a list or tuple")
    if not isinstance(intervals, (list, tuple)):
        raise ValueError("intervals must be a list or tuple")
    if isinstance(exclusions, bool):
        raise ValueError("exclusions must be a list or tuple")
    if not isinstance(exclusions, (list, tuple)):
        raise ValueError("exclusions must be a list or tuple")
    ivs = [_validate_interval(i) for i in intervals]
    exs = [_validate_interval(i) for i in exclusions]
    if not ivs:
        return []
    base = normalize(ivs)
    if not exs:
        return base
    ex_norm = normalize(exs)
    res = []
    for s, e in base:
        cur = s
        for xs, xe in ex_norm:
            if xe <= cur:
                continue
            if xs >= e:
                break
            if xs > cur:
                res.append((cur, xs))
            cur = max(cur, xe)
            if cur >= e:
                break
        if cur < e:
            res.append((cur, e))
    return res

def earliest_slot(busy, duration, window):
    if isinstance(busy, bool):
        raise ValueError("busy must be a list or tuple")
    if not isinstance(busy, (list, tuple)):
        raise ValueError("busy must be a list or tuple")
    if isinstance(duration, bool):
        raise ValueError("duration must be a positive integer")
    if not isinstance(duration, int):
        raise ValueError("duration must be a positive integer")
    if duration <= 0:
        raise ValueError("duration must be a positive integer")
    if isinstance(window, bool):
        raise ValueError("window must be a list or tuple of two integers")
    if not isinstance(window, (list, tuple)):
        raise ValueError("window must be a list or tuple of two integers")
    if len(window) != 2:
        raise ValueError("window must have exactly two elements")
    ws, we = window
    if isinstance(ws, bool) or isinstance(we, bool):
        raise ValueError("window coordinates must be integers, not booleans")
    if not isinstance(ws, int) or not isinstance(we, int):
        raise ValueError("window coordinates must be integers")
    if not ws < we:
        raise ValueError("window start must be less than end")
    busy_ivs = [_validate_interval(i) for i in busy]
    if we - ws < duration:
        return None
    busy_norm = normalize(busy_ivs)
    cur = ws
    for bs, be in busy_norm:
        if be <= cur:
            continue
        if bs >= we:
            break
        if bs > cur:
            if bs - cur >= duration:
                return cur
            cur = bs
        cur = max(cur, be)
        if cur >= we:
            break
    if we - cur >= duration:
        return cur
    return None