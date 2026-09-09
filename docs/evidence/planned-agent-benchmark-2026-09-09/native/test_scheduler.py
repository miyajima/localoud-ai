import copy
import random
import unittest

from scheduler import topological_order, execution_batches, critical_path_length


def task(deps=(), duration=1, group=None):
    return dict(deps=list(deps), duration=duration, group=group)


class SchedulerTests(unittest.TestCase):
    def test_priority_and_batch_boundary(self):
        tasks = {'a': task(), 'b': task(['a']), 'c': task(['b']), 'z': task()}
        self.assertEqual(topological_order(tasks), ['a', 'b', 'c', 'z'])
        self.assertEqual(execution_batches(tasks, 3), [['a', 'z'], ['b'], ['c']])

    def test_conflicts_and_unrestricted_groups(self):
        tasks = {key: task(group=group) for key, group in
                 [('a', 'x'), ('b', 'x'), ('c', None), ('d', None), ('e', 'y')]}
        self.assertEqual(execution_batches(tasks, 4), [['a', 'c', 'd', 'e'], ['b']])

    def test_duplicates_weighted_path_and_immutability(self):
        tasks = {'a': task(duration=4, group='x'), 'b': task(['a', 'a'], 3, 'x'),
                 'c': task(['a'], 7), 'd': task(['b', 'c'], 2)}
        before = copy.deepcopy(tasks)
        self.assertEqual(topological_order(tasks), ['a', 'b', 'c', 'd'])
        self.assertEqual(execution_batches(tasks, 3), [['a'], ['b', 'c'], ['d']])
        self.assertEqual(critical_path_length(tasks), 13)
        self.assertEqual(tasks, before)

    def test_invalid_inputs_all_operations(self):
        invalid = [None, (), {'a': None}, {'a': task([{}])}, {'a': task([True])},
                   {'a': task([''])}, {'a': task(['unknown'])}, {'a': task(['a'])},
                   {'a': task(duration=False)}, {'a': task(duration=-1)},
                   {'a': task(group=[])}, {'a': dict(task(), deps=())},
                   {'a': task(), 'x': task(['y']), 'y': task(['x'])}]
        for tasks in invalid:
            for function in (topological_order, critical_path_length,
                             lambda value: execution_batches(value, 2)):
                with self.subTest(tasks=tasks, function=function), self.assertRaises(ValueError):
                    function(tasks)

    def test_invalid_capacity_nonempty(self):
        for capacity in (True, False, -2, 0, 2.0, [], '3'):
            with self.subTest(capacity=capacity), self.assertRaises(ValueError):
                execution_batches({'a': task()}, capacity)

    def test_generated_dags(self):
        rng = random.Random(47)
        for size in range(1, 16):
            ids = [f't{i:02d}' for i in range(size)]
            tasks = {key: task([dep for dep in ids[:index] if rng.random() < .3],
                               rng.randrange(1, 10), rng.choice([None, 'x', 'y']))
                     for index, key in enumerate(ids)}
            completed, expected_order, finishes = set(), [], {}
            while len(completed) < size:
                key = min(key for key in tasks if key not in completed
                          and set(tasks[key]['deps']) <= completed)
                completed.add(key)
                expected_order.append(key)
                finishes[key] = tasks[key]['duration'] + max(
                    [finishes[dep] for dep in tasks[key]['deps']], default=0)
            self.assertEqual(topological_order(tasks), expected_order)
            self.assertEqual(critical_path_length(tasks), max(finishes.values()))
            completed = set()
            for batch in execution_batches(tasks, 3):
                self.assertTrue(0 < len(batch) <= 3)
                self.assertEqual(batch, sorted(batch))
                self.assertTrue(all(set(tasks[key]['deps']) <= completed for key in batch))
                self.assertTrue(completed.isdisjoint(batch))
                groups = [tasks[key]['group'] for key in batch if tasks[key]['group'] is not None]
                self.assertEqual(len(groups), len(set(groups)))
                completed.update(batch)
            self.assertEqual(completed, set(tasks))


if __name__ == '__main__':
    unittest.main()
