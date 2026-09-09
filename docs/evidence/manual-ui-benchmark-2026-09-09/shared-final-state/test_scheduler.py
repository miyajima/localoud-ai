import copy
import unittest

from scheduler import critical_path_length, execution_batches, topological_order


def task(deps=(), duration=1, group=None):
    return {"deps": list(deps), "duration": duration, "group": group}


class SchedulerTests(unittest.TestCase):
    def test_newly_ready_task_has_lexicographic_priority(self):
        tasks = {"a": task(), "b": task(["a"]), "z": task()}
        self.assertEqual(topological_order(tasks), ["a", "b", "z"])

    def test_batches_wait_for_tasks_enabled_by_previous_batch(self):
        tasks = {"a": task(), "b": task(["a"]), "z": task()}
        self.assertEqual(execution_batches(tasks, 2), [["a", "z"], ["b"]])

    def test_group_conflicts_are_skipped_to_fill_capacity(self):
        tasks = {
            "a": task(group="x"),
            "b": task(group="x"),
            "c": task(group="y"),
            "d": task(),
        }
        self.assertEqual(execution_batches(tasks, 3), [["a", "c", "d"], ["b"]])

    def test_duplicate_dependencies_are_deduplicated(self):
        tasks = {"a": task(), "b": task(["a", "a"], duration=2)}
        self.assertEqual(topological_order(tasks), ["a", "b"])
        self.assertEqual(execution_batches(tasks, 2), [["a"], ["b"]])
        self.assertEqual(critical_path_length(tasks), 3)

    def test_disconnected_cycle_is_rejected(self):
        tasks = {"a": task(), "b": task(["c"]), "c": task(["b"])}
        for function in (
            topological_order,
            critical_path_length,
            lambda value: execution_batches(value, 2),
        ):
            with self.subTest(function=function):
                with self.assertRaises(ValueError):
                    function(tasks)

    def test_malformed_schema_is_rejected_as_value_error(self):
        malformed = [
            None,
            [],
            {1: task()},
            {"": task()},
            {"a": {}},
            {"a": task(duration=True)},
            {"a": task(duration=0)},
            {"a": task(duration=1.5)},
            {"a": task(group="")},
            {"a": task(group=2)},
            {"a": dict(task(), extra=1)},
            {"a": dict(task(), deps="a")},
            {"a": dict(task(), deps=[[]])},
        ]
        for value in malformed:
            for function in (
                topological_order,
                critical_path_length,
                lambda item: execution_batches(item, 2),
            ):
                with self.subTest(value=value, function=function):
                    with self.assertRaises(ValueError):
                        function(value)

    def test_capacity_rejects_bool_and_other_nonpositive_values(self):
        for capacity in (0, -1, True, 1.5, None, "2"):
            with self.subTest(capacity=capacity):
                with self.assertRaises(ValueError):
                    execution_batches({}, capacity)

    def test_input_is_immutable(self):
        tasks = {"b": task(["a", "a"], duration=2, group="x"), "a": task()}
        before = copy.deepcopy(tasks)
        topological_order(tasks)
        execution_batches(tasks, 2)
        critical_path_length(tasks)
        self.assertEqual(tasks, before)

    def test_weighted_critical_path_ignores_groups(self):
        tasks = {
            "a": task(duration=2),
            "b": task(["a"], duration=3, group="x"),
            "c": task(["a"], duration=5, group="x"),
            "d": task(["b", "c"], duration=4),
            "z": task(duration=20),
        }
        self.assertEqual(critical_path_length(tasks), 20)
        tasks.pop("z")
        self.assertEqual(critical_path_length(tasks), 11)

    def test_empty_graph_and_deterministic_capacity(self):
        self.assertEqual(topological_order({}), [])
        self.assertEqual(execution_batches({}, 1), [])
        self.assertEqual(critical_path_length({}), 0)
        self.assertEqual(
            execution_batches({name: task() for name in "edcba"}, 2),
            [["a", "b"], ["c", "d"], ["e"]],
        )


if __name__ == "__main__":
    unittest.main()
