"""Deterministic dependency scheduling with validated task descriptions."""

import heapq


def _validated(tasks):
    if not isinstance(tasks, dict):
        raise ValueError('tasks must be a dict')
    if any(not isinstance(key, str) or not key for key in tasks):
        raise ValueError('task IDs must be nonempty strings')
    dependencies = {}
    for key, task in tasks.items():
        if not isinstance(task, dict) or set(task) != {'deps', 'duration', 'group'}:
            raise ValueError('invalid task fields')
        deps, duration, group = task['deps'], task['duration'], task['group']
        if not isinstance(duration, int) or isinstance(duration, bool) or duration <= 0:
            raise ValueError('duration must be a positive integer')
        if group is not None and (not isinstance(group, str) or not group):
            raise ValueError('group must be None or a nonempty string')
        if not isinstance(deps, list):
            raise ValueError('deps must be a list')
        for dep in deps:
            if not isinstance(dep, str) or not dep or dep not in tasks or dep == key:
                raise ValueError('invalid dependency')
        dependencies[key] = set(deps)
    successors = {key: [] for key in tasks}
    remaining = {key: len(deps) for key, deps in dependencies.items()}
    for key, deps in dependencies.items():
        for dep in deps:
            successors[dep].append(key)
    ready = [key for key, count in remaining.items() if count == 0]
    heapq.heapify(ready)
    order = []
    while ready:
        key = heapq.heappop(ready)
        order.append(key)
        for successor in successors[key]:
            remaining[successor] -= 1
            if remaining[successor] == 0:
                heapq.heappush(ready, successor)
    if len(order) != len(tasks):
        raise ValueError('dependency cycle')
    return dependencies, order


def topological_order(tasks):
    """Return the smallest currently ready ID at each step."""
    return _validated(tasks)[1]


def execution_batches(tasks, max_parallel):
    """Greedily fill each batch using completed dependencies and group limits."""
    if (not isinstance(max_parallel, int) or isinstance(max_parallel, bool)
            or max_parallel <= 0):
        raise ValueError('max_parallel must be a positive integer')
    dependencies, _ = _validated(tasks)
    pending, completed, batches = set(tasks), set(), []
    while pending:
        ready = sorted(key for key in pending if dependencies[key] <= completed)
        batch, groups = [], set()
        for key in ready:
            group = tasks[key]['group']
            if group is not None and group in groups:
                continue
            batch.append(key)
            if group is not None:
                groups.add(group)
            if len(batch) == max_parallel:
                break
        batches.append(batch)
        completed.update(batch)
        pending.difference_update(batch)
    return batches


def critical_path_length(tasks):
    """Return the longest duration-weighted dependency path."""
    dependencies, order = _validated(tasks)
    finish = {}
    for key in order:
        finish[key] = tasks[key]['duration'] + max(
            (finish[dep] for dep in dependencies[key]), default=0)
    return max(finish.values(), default=0)
