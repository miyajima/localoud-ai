"""Deterministic dependency scheduling utilities."""

from heapq import heappop, heappush


_TASK_FIELDS = {"deps", "duration", "group"}


def _validate_tasks(tasks):
    """Validate and copy a task graph into internal immutable-by-convention data."""
    if not isinstance(tasks, dict):
        raise ValueError("tasks must be a dict")

    task_ids = list(tasks.keys())
    for task_id in task_ids:
        if not isinstance(task_id, str) or not task_id:
            raise ValueError("task IDs must be nonempty strings")

    dependencies = {}
    durations = {}
    groups = {}

    # Validate every node and every field before checking dependency membership.
    # This keeps malformed or otherwise unhashable dependency values on the
    # documented ValueError path.
    for task_id in task_ids:
        task = tasks[task_id]
        if not isinstance(task, dict) or set(task.keys()) != _TASK_FIELDS:
            raise ValueError("each task must have exactly deps, duration, and group")

        raw_deps = task["deps"]
        if not isinstance(raw_deps, list):
            raise ValueError("deps must be a list")
        copied_deps = []
        for dependency in raw_deps:
            if not isinstance(dependency, str) or not dependency:
                raise ValueError("dependencies must be nonempty strings")
            copied_deps.append(dependency)

        duration = task["duration"]
        if isinstance(duration, bool) or not isinstance(duration, int) or duration <= 0:
            raise ValueError("duration must be a positive integer")

        group = task["group"]
        if group is not None and (not isinstance(group, str) or not group):
            raise ValueError("group must be None or a nonempty string")

        dependencies[task_id] = set(copied_deps)
        durations[task_id] = duration
        groups[task_id] = group

    known_ids = set(task_ids)
    for task_id, task_dependencies in dependencies.items():
        if task_id in task_dependencies:
            raise ValueError("a task cannot depend on itself")
        if not task_dependencies.issubset(known_ids):
            raise ValueError("dependency refers to an unknown task")

    successors = {task_id: set() for task_id in task_ids}
    for task_id, task_dependencies in dependencies.items():
        for dependency in task_dependencies:
            successors[dependency].add(task_id)

    return dependencies, durations, groups, successors


def _topological_from_graph(dependencies, successors):
    """Return the lexicographically minimal topological order."""
    indegree = {task_id: len(task_dependencies) for task_id, task_dependencies in dependencies.items()}
    ready = [task_id for task_id, degree in indegree.items() if degree == 0]
    ready.sort()
    order = []

    while ready:
        task_id = heappop(ready)
        order.append(task_id)
        for successor in successors[task_id]:
            indegree[successor] -= 1
            if indegree[successor] == 0:
                heappush(ready, successor)

    if len(order) != len(dependencies):
        raise ValueError("task graph contains a cycle")
    return order


def topological_order(tasks):
    """Return the lexicographically minimal valid topological order."""
    dependencies, _durations, _groups, successors = _validate_tasks(tasks)
    return _topological_from_graph(dependencies, successors)


def _validate_capacity(max_parallel):
    if isinstance(max_parallel, bool) or not isinstance(max_parallel, int) or max_parallel <= 0:
        raise ValueError("max_parallel must be a positive integer")


def execution_batches(tasks, max_parallel):
    """Build deterministic dependency-respecting execution batches."""
    _validate_capacity(max_parallel)
    dependencies, _durations, groups, successors = _validate_tasks(tasks)
    # Validate cycles independently of the batch readiness semantics.
    _topological_from_graph(dependencies, successors)

    pending = set(dependencies)
    completed = set()
    batches = []

    while pending:
        ready = sorted(
            task_id
            for task_id in pending
            if dependencies[task_id].issubset(completed)
        )
        selected = []
        used_groups = set()
        for task_id in ready:
            group = groups[task_id]
            if group is not None and group in used_groups:
                continue
            selected.append(task_id)
            if group is not None:
                used_groups.add(group)
            if len(selected) == max_parallel:
                break

        # A validated acyclic graph always has a ready task while pending is
        # nonempty. Keep this guard so the loop cannot silently stall if that
        # invariant is ever broken by a future change.
        if not selected:
            raise ValueError("task graph contains a cycle")

        completed.update(selected)
        pending.difference_update(selected)
        batches.append(selected)

    return batches


def critical_path_length(tasks):
    """Return the largest dependency-path duration in the task graph."""
    dependencies, durations, _groups, successors = _validate_tasks(tasks)
    order = _topological_from_graph(dependencies, successors)
    finish = {}
    for task_id in order:
        predecessor_finish = max(
            (finish[dependency] for dependency in dependencies[task_id]),
            default=0,
        )
        finish[task_id] = predecessor_finish + durations[task_id]
    return max(finish.values(), default=0)
