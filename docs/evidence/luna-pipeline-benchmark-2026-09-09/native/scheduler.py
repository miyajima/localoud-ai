"""Deterministic dependency scheduling utilities."""

from heapq import heapify, heappop, heappush


_TASK_FIELDS = frozenset(("deps", "duration", "group"))


def _validate_tasks(tasks):
    """Validate and normalize tasks, returning graph data and a topo order."""
    if not isinstance(tasks, dict):
        raise ValueError("tasks must be a dict")

    node_ids = set()
    for node_id in tasks:
        if not isinstance(node_id, str) or len(node_id) == 0:
            raise ValueError("task IDs must be nonempty strings")
        node_ids.add(node_id)

    normalized = {}
    for node_id, task in tasks.items():
        if not isinstance(task, dict):
            raise ValueError("each task must be a dict")
        try:
            fields = set(task.keys())
        except Exception as exc:
            raise ValueError("task fields are invalid") from exc
        if fields != _TASK_FIELDS:
            raise ValueError("each task must have exactly deps, duration, and group")

        deps = task["deps"]
        duration = task["duration"]
        group = task["group"]

        if not isinstance(deps, list):
            raise ValueError("deps must be a list")
        validated_deps = []
        for dependency in deps:
            if not isinstance(dependency, str) or len(dependency) == 0:
                raise ValueError("dependency IDs must be nonempty strings")
            try:
                hash(dependency)
            except Exception as exc:
                raise ValueError("dependency IDs must be hashable") from exc
            validated_deps.append(dependency)

        if isinstance(duration, bool) or not isinstance(duration, int) or duration <= 0:
            raise ValueError("duration must be a positive integer")

        if group is not None:
            if not isinstance(group, str) or len(group) == 0:
                raise ValueError("group must be None or a nonempty string")
            try:
                hash(group)
            except Exception as exc:
                raise ValueError("group must be hashable") from exc

        normalized[node_id] = {
            "deps": set(validated_deps),
            "duration": duration,
            "group": group,
        }

    for node_id, task in normalized.items():
        for dependency in task["deps"]:
            if dependency not in node_ids or dependency == node_id:
                raise ValueError("dependency must refer to another known task")

    successors = {node_id: set() for node_id in node_ids}
    indegree = {}
    for node_id, task in normalized.items():
        indegree[node_id] = len(task["deps"])
        for dependency in task["deps"]:
            successors[dependency].add(node_id)

    ready = [node_id for node_id, degree in indegree.items() if degree == 0]
    heapify(ready)
    order = []
    while ready:
        node_id = heappop(ready)
        order.append(node_id)
        for successor in successors[node_id]:
            indegree[successor] -= 1
            if indegree[successor] == 0:
                heappush(ready, successor)

    if len(order) != len(node_ids):
        raise ValueError("tasks contain a cycle")

    return normalized, order


def _validate_capacity(max_parallel):
    if isinstance(max_parallel, bool) or not isinstance(max_parallel, int) or max_parallel <= 0:
        raise ValueError("max_parallel must be a positive integer")


def topological_order(tasks):
    """Return the lexicographically minimal valid topological order."""
    _, order = _validate_tasks(tasks)
    return order


def execution_batches(tasks, max_parallel):
    """Build deterministic greedy batches subject to capacity and groups."""
    _validate_capacity(max_parallel)
    normalized, _ = _validate_tasks(tasks)

    pending = set(normalized)
    completed = set()
    batches = []

    while pending:
        ready = sorted(
            node_id
            for node_id in pending
            if normalized[node_id]["deps"] <= completed
        )
        batch = []
        groups = set()
        for node_id in ready:
            group = normalized[node_id]["group"]
            if group is not None and group in groups:
                continue
            batch.append(node_id)
            if group is not None:
                groups.add(group)
            if len(batch) == max_parallel:
                break

        if not batch:
            raise ValueError("unable to make progress")
        batches.append(batch)
        completed.update(batch)
        pending.difference_update(batch)

    return batches


def critical_path_length(tasks):
    """Return the largest duration sum along any dependency path."""
    normalized, order = _validate_tasks(tasks)
    finish = {}
    for node_id in order:
        predecessors = normalized[node_id]["deps"]
        predecessor_finish = max(
            (finish[dependency] for dependency in predecessors),
            default=0,
        )
        finish[node_id] = predecessor_finish + normalized[node_id]["duration"]
    return max(finish.values(), default=0)
