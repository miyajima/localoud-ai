import itertools
import unittest

from intervals import contains, normalize, subtract


class IntervalTests(unittest.TestCase):
    def test_empty_and_duplicate_intervals(self):
        self.assertEqual(normalize([]), [])
        self.assertEqual(normalize([(2, 2), (1, 5), (2, 3), (1, 5)]), [(1, 5)])

    def test_cut_spans_separate_sources(self):
        self.assertEqual(subtract([(0, 2), (4, 6), (8, 10)], [(1, 9)]), [(0, 1), (9, 10)])

    def test_touching_cuts_leave_sources(self):
        self.assertEqual(subtract([(1, 3)], [(-1, 1), (3, 5), (2, 2)]), [(1, 3)])

    def test_no_cuts_normalizes(self):
        self.assertEqual(subtract([(3, 5), (1, 3)], []), [(1, 5)])

    def test_all_functions_validate_reversed_intervals(self):
        for operation in (
            lambda: contains([(0, 3), (5, 4)], 1),
            lambda: subtract([], [(5, 4)]),
            lambda: subtract([(5, 4)], []),
        ):
            with self.assertRaises(ValueError):
                operation()

    def test_no_mutation_with_mutable_pairs(self):
        sources = [[4, 6], [0, 3]]
        cuts = [[5, 7], [1, 2]]
        normalize(sources)
        contains(sources, 1)
        subtract(sources, cuts)
        self.assertEqual(sources, [[4, 6], [0, 3]])
        self.assertEqual(cuts, [[5, 7], [1, 2]])

    def test_fractional_bounds_and_generators(self):
        self.assertEqual(subtract(iter([(0.5, 2.5)]), iter([(1.0, 2.0)])), [(0.5, 1.0), (2.0, 2.5)])
        self.assertFalse(contains([(1, 1)], 1))

    def test_exhaustive_small_integer_sets(self):
        intervals = list(itertools.combinations_with_replacement(range(-2, 3), 2))
        sources = [[], *([interval] for interval in intervals)]
        sources += [list(pair) for pair in itertools.combinations(intervals, 2)]
        for original in sources:
            for cut in intervals:
                result = subtract(original, [cut])
                self.assertEqual(result, normalize(result))
                for point in (value / 2 for value in range(-5, 6)):
                    expected = any(a <= point < b for a, b in original) and not cut[0] <= point < cut[1]
                    self.assertEqual(contains(result, point), expected, (original, cut, point))


if __name__ == "__main__":
    unittest.main()
