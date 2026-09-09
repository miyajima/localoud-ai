import copy
import unittest

from scheduler import critical_path_length, execution_batches, topological_order


def task(deps=(), duration=1, group=None):
    return {"deps": list(deps), "duration": duration, "group": group}


class SchedulerTests(unittest.TestCase):
    def test_topological_order_uses_newly_ready_lexicographic_priority(self):
        tasks = {
            "a": task(),
            "b": task(["a"]),
            "aa": task(["b"]),
            "z": task(),
        }
        self.assertEqual(topological_order(tasks), ["a", "b", "aa", "z"])

    def test_execution_batches_delay_newly_ready_tasks(self):
        tasks = {"a": task(), "b": task(), "c": task(["a"])}
        self.assertEqual(execution_batches(tasks, 2), [["a", "b"], ["c"]])

    def test_execution_batches_skip_group_conflicts_and_fill_capacity(self):
        tasks = {
            "a": task(group="same"),
            "b": task(group="same"),
            "c": task(group="other"),
            "d": task(),
        }
        self.assertEqual(execution_batches(tasks, 3), [["a", "c", "d"], ["b"]])

    def test_duplicate_dependencies_are_deduplicated(self):
        tasks = {"a": task(), "b": task(["a", "a"], duration=4)}
        self.assertEqual(topological_order(tasks), ["a", "b"])
        self.assertEqual(execution_batches(tasks, 2), [["a"], ["b"]])
        self.assertEqual(critical_path_length(tasks), 5)

    def test_disconnected_cycle_is_rejected_by_every_operation(self):
        tasks = {"ready": task(), "a": task(["b"]), "b": task(["a"])}
        functions = (
            topological_order,
            critical_path_length,
            lambda value: execution_batches(value, 2),
        )
        for function in functions:
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
            {"a": {"deps": [["b"]], "duration": 1, "group": None}},
            {"a": {"deps": [], "duration": 1, "group": ""}},
            {"a": {"deps": [], "duration": 1, "group": 3}},
            {"a": {"deps": [], "duration": 1, "group": None, "extra": 1}},
        ]
        functions = (
            topological_order,
            critical_path_length,
            lambda value: execution_batches(value, 1),
        )
        for value in malformed:
            for function in functions:
                with self.subTest(value=value, function=function):
                    with self.assertRaises(ValueError):
                        function(value)

    def test_bool_duration_and_capacity_are_rejected(self):
        with self.assertRaises(ValueError):
            topological_order({"a": task(duration=True)})
        with self.assertRaises(ValueError):
            critical_path_length({"a": task(duration=False)})
        with self.assertRaises(ValueError):
            execution_batches({"a": task()}, True)
        with self.assertRaises(ValueError):
            execution_batches({}, False)

    def test_input_is_immutable(self):
        tasks = {"a": task(), "b": task(["a", "a"], 2, "g")}
        before = copy.deepcopy(tasks)
        topological_order(tasks)
        execution_batches(tasks, 2)
        critical_path_length(tasks)
        self.assertEqual(tasks, before)

    def test_weighted_critical_path_ignores_groups(self):
        tasks = {
            "a": task(duration=2, group="x"),
            "b": task(["a"], duration=3, group="x"),
            "c": task(["a"], duration=7, group="y"),
            "d": task(["b", "c"], duration=4),
            "independent": task(duration=20, group="x"),
        }
        self.assertEqual(critical_path_length(tasks), 20)
        tasks.pop("independent")
        self.assertEqual(critical_path_length(tasks), 13)

    def test_empty_graph_and_capacity_validation(self):
        self.assertEqual(topological_order({}), [])
        self.assertEqual(execution_batches({}, 1), [])
        self.assertEqual(critical_path_length({}), 0)
        for capacity in (0, -1, 1.5, None, "1", True):
            with self.subTest(capacity=capacity):
                with self.assertRaises(ValueError):
                    execution_batches({}, capacity)


if __name__ == "__main__":
    unittest.main()
