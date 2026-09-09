"""Dependency-aware scheduling helpers."""

import heapq


_FIELDS = frozenset(("deps", "duration", "group"))


def _validate_tasks(tasks):
    """Validate and copy a task graph without modifying the caller's data."""
    if not isinstance(tasks, dict):
        raise ValueError("tasks must be a dict")

    task_ids = list(tasks)
    if any(not isinstance(task_id, str) or not task_id for task_id in task_ids):
        raise ValueError("task IDs must be nonempty strings")

    try:
        known_ids = set(task_ids)
    except TypeError as exc:
        raise ValueError("task IDs must be hashable strings") from exc

    dependencies = {}
    durations = {}
    groups = {}

    for task_id, task in tasks.items():
        if not isinstance(task, dict) or set(task) != _FIELDS:
            raise ValueError("each task must have exactly deps, duration, and group")

        deps = task["deps"]
        duration = task["duration"]
        group = task["group"]

        if not isinstance(deps, list):
            raise ValueError("deps must be a list")
        if isinstance(duration, bool) or not isinstance(duration, int) or duration <= 0:
            raise ValueError("duration must be a positive integer")
        if group is not None and (not isinstance(group, str) or not group):
            raise ValueError("group must be None or a nonempty string")

        copied_deps = set()
        for dependency in deps:
            if not isinstance(dependency, str) or not dependency:
                raise ValueError("dependencies must be nonempty strings")
            try:
                known_dependency = dependency in known_ids
            except TypeError as exc:
                raise ValueError("dependencies must be hashable strings") from exc
            if not known_dependency:
                raise ValueError("dependency refers to an unknown task")
            if dependency == task_id:
                raise ValueError("a task cannot depend on itself")
            copied_deps.add(dependency)

        dependencies[task_id] = copied_deps
        durations[task_id] = duration
        groups[task_id] = group

    return dependencies, durations, groups


def _topological_order_from_dependencies(dependencies):
    """Return a lexicographically minimal order, or reject a cycle."""
    indegree = {task_id: len(deps) for task_id, deps in dependencies.items()}
    successors = {task_id: [] for task_id in dependencies}
    for task_id, deps in dependencies.items():
        for dependency in deps:
            successors[dependency].append(task_id)

    ready = [task_id for task_id, degree in indegree.items() if degree == 0]
    heapq.heapify(ready)
    order = []

    while ready:
        task_id = heapq.heappop(ready)
        order.append(task_id)
        for successor in successors[task_id]:
            indegree[successor] -= 1
            if indegree[successor] == 0:
                heapq.heappush(ready, successor)

    if len(order) != len(dependencies):
        raise ValueError("task graph contains a cycle")
    return order


def _validate_capacity(max_parallel):
    if (
        isinstance(max_parallel, bool)
        or not isinstance(max_parallel, int)
        or max_parallel <= 0
    ):
        raise ValueError("max_parallel must be a positive integer")


def topological_order(tasks):
    """Return all task IDs in lexicographically minimal topological order."""
    dependencies, _, _ = _validate_tasks(tasks)
    return _topological_order_from_dependencies(dependencies)


def execution_batches(tasks, max_parallel):
    """Return deterministic greedy execution batches."""
    _validate_capacity(max_parallel)
    dependencies, _, groups = _validate_tasks(tasks)

    pending = set(dependencies)
    completed = set()
    batches = []

    while pending:
        ready = sorted(
            task_id
            for task_id in pending
            if dependencies[task_id].issubset(completed)
        )

        if not ready:
            raise ValueError("task graph contains a cycle")

        batch = []
        selected_groups = []
        for task_id in ready:
            group = groups[task_id]
            if group is not None and group in selected_groups:
                continue
            batch.append(task_id)
            if group is not None:
                selected_groups.append(group)
            if len(batch) == max_parallel:
                break

        completed.update(batch)
        pending.difference_update(batch)
        batches.append(batch)

    return batches


def critical_path_length(tasks):
    """Return the longest weighted dependency path."""
    dependencies, durations, _ = _validate_tasks(tasks)
    order = _topological_order_from_dependencies(dependencies)
    finish_times = {}

    for task_id in order:
        predecessor_finish = max(
            (finish_times[dependency] for dependency in dependencies[task_id]),
            default=0,
        )
        finish_times[task_id] = predecessor_finish + durations[task_id]

    return max(finish_times.values(), default=0)
