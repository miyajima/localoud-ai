"""Deterministic dependency scheduling with validated task schemas."""

import heapq


def _validate(tasks):
    if not isinstance(tasks, dict):
        raise ValueError("tasks must be a dict")
    if any(not isinstance(name, str) or not name for name in tasks):
        raise ValueError("task IDs must be nonempty strings")

    graph = {}
    for name, task in tasks.items():
        if not isinstance(task, dict) or set(task) != {"deps", "duration", "group"}:
            raise ValueError("tasks require exactly deps, duration, and group")
        deps, duration, group = task["deps"], task["duration"], task["group"]
        if not isinstance(duration, int) or isinstance(duration, bool) or duration <= 0:
            raise ValueError("duration must be a positive integer excluding bool")
        if group is not None and (not isinstance(group, str) or not group):
            raise ValueError("group must be None or a nonempty string")
        if not isinstance(deps, list):
            raise ValueError("deps must be a list")
        for dep in deps:
            if not isinstance(dep, str) or not dep:
                raise ValueError("dependencies must be nonempty strings")
            if dep not in tasks or dep == name:
                raise ValueError("unknown or self dependency")
        graph[name] = (set(deps), duration, group)

    successors = {name: [] for name in graph}
    remaining = {}
    for name, (deps, _, _) in graph.items():
        remaining[name] = len(deps)
        for dep in deps:
            successors[dep].append(name)
    ready = [name for name in graph if remaining[name] == 0]
    heapq.heapify(ready)
    order = []
    while ready:
        name = heapq.heappop(ready)
        order.append(name)
        for successor in successors[name]:
            remaining[successor] -= 1
            if remaining[successor] == 0:
                heapq.heappush(ready, successor)
    if len(order) != len(graph):
        raise ValueError("dependency cycle")
    return graph, order


def topological_order(tasks):
    """Return the lexicographically smallest dependency-respecting order."""
    _, order = _validate(tasks)
    return order


def execution_batches(tasks, max_parallel):
    """Greedily batch ready tasks subject to capacity and group conflicts."""
    if (not isinstance(max_parallel, int) or isinstance(max_parallel, bool)
            or max_parallel <= 0):
        raise ValueError("max_parallel must be a positive integer excluding bool")
    graph, _ = _validate(tasks)
    completed, pending = set(), set(graph)
    batches = []
    while pending:
        ready = sorted(name for name in pending if graph[name][0] <= completed)
        batch, groups = [], set()
        for name in ready:
            group = graph[name][2]
            if group is not None and group in groups:
                continue
            batch.append(name)
            if group is not None:
                groups.add(group)
            if len(batch) == max_parallel:
                break
        batches.append(batch)
        completed.update(batch)
        pending.difference_update(batch)
    return batches


def critical_path_length(tasks):
    """Return the longest duration sum along a dependency path."""
    graph, order = _validate(tasks)
    finish = {}
    for name in order:
        deps, duration, _ = graph[name]
        finish[name] = duration + max((finish[dep] for dep in deps), default=0)
    return max(finish.values(), default=0)
