import copy
import itertools
import unittest

from scheduler import critical_path_length, execution_batches, topological_order


def task(deps=(), duration=1, group=None):
    return {"deps": list(deps), "duration": duration, "group": group}


class SchedulerTests(unittest.TestCase):
    def assert_invalid(self, tasks):
        for function in (topological_order, critical_path_length,
                         lambda value: execution_batches(value, 2)):
            with self.subTest(function=function), self.assertRaises(ValueError):
                function(tasks)

    def test_newly_ready_priority_and_batch_boundary(self):
        tasks = {"z": task(), "c": task(["b"]), "b": task(["a"]), "a": task()}
        self.assertEqual(topological_order(tasks), ["a", "b", "c", "z"])
        self.assertEqual(execution_batches(tasks, 2), [["a", "z"], ["b"], ["c"]])

    def test_conflicts_skip_and_none_is_unrestricted(self):
        tasks = {"a": task(group="x"), "b": task(group="x"),
                 "c": task(group="x"), "d": task(group="y"),
                 "e": task(), "f": task()}
        self.assertEqual(execution_batches(tasks, 4),
                         [["a", "d", "e", "f"], ["b"], ["c"]])

    def test_duplicates_and_immutability(self):
        tasks = {"b": task(["a", "a"], 3, "x"), "a": task(duration=2)}
        before = copy.deepcopy(tasks)
        for _ in range(2):
            self.assertEqual(topological_order(tasks), ["a", "b"])
            self.assertEqual(execution_batches(tasks, 3), [["a"], ["b"]])
            self.assertEqual(critical_path_length(tasks), 5)
        self.assertEqual(tasks, before)

    def test_cycles_and_unknown_dependencies(self):
        for tasks in ({"a": task(["a"])}, {"a": task(["missing"])},
                      {"a": task(), "b": task(["c"]), "c": task(["b"])}):
            self.assert_invalid(tasks)

    def test_malformed_schemas(self):
        for tasks in (None, [], (), {False: task()}, {"": task()},
                      {"a": None}, {"a": []}, {"a": {}},
                      {"a": dict(task(), extra=1)}):
            self.assert_invalid(tasks)
        for field, values in {
            "deps": (None, (), "a", [None], [False], [1], [""], [[]], [{}]),
            "duration": (None, False, True, 0, -1, 1.5, "1", []),
            "group": ("", False, 1, [], {}),
        }.items():
            for value in values:
                with self.subTest(field=field, value=value):
                    self.assert_invalid({"valid": task(), "bad": dict(task(), **{field: value})})

    def test_capacity_validation_including_empty(self):
        for capacity in (False, True, 0, -1, 1.5, None, "2", [], {}):
            for tasks in ({}, {"a": task()}):
                with self.subTest(capacity=capacity), self.assertRaises(ValueError):
                    execution_batches(tasks, capacity)

    def test_weighted_paths_ignore_groups(self):
        tasks = {"a": task(duration=2, group="x"),
                 "b": task(["a"], 7, "x"), "c": task(["a"], 3, "x"),
                 "d": task(["b", "c"], 4, "x"), "z": task(duration=12)}
        self.assertEqual(critical_path_length(tasks), 13)
        self.assertEqual(critical_path_length({}), 0)
        self.assertEqual(topological_order({}), [])
        self.assertEqual(execution_batches({}, 1), [])

    def test_generated_dags_against_exhaustive_orders_and_paths(self):
        names = ("d", "b", "a", "c")
        edges = list(itertools.combinations(names, 2))
        for mask in range(1 << len(edges)):
            tasks = {name: task(duration=i + 1) for i, name in enumerate(names)}
            for bit, (source, target) in enumerate(edges):
                if mask & (1 << bit):
                    tasks[target]["deps"].append(source)
            valid_orders = [order for order in itertools.permutations(names)
                            if all(order.index(dep) < order.index(name)
                                   for name in names for dep in tasks[name]["deps"])]
            self.assertEqual(topological_order(tasks), list(min(valid_orders)))

            def path_lengths(name):
                duration = tasks[name]["duration"]
                return [duration] + [duration + length
                                     for dep in tasks[name]["deps"]
                                     for length in path_lengths(dep)]

            self.assertEqual(critical_path_length(tasks),
                             max(length for name in names for length in path_lengths(name)))


if __name__ == "__main__":
    unittest.main()
