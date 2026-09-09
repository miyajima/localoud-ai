import copy
import itertools
import unittest

from intervals import contains, normalize, subtract


class IntervalTests(unittest.TestCase):
    def test_empty_and_degenerate(self):
        self.assertEqual(normalize([]), [])
        self.assertEqual(normalize([(2, 2), (-1, -1)]), [])
        self.assertEqual(subtract([(0, 4)], [(2, 2)]), [(0, 4)])
        self.assertFalse(contains([(2, 2)], 2))

    def test_nested_duplicate_and_touching(self):
        self.assertEqual(
            normalize([(5, 6), (0, 4), (1, 2), (0, 4), (4, 5)]),
            [(0, 6)],
        )

    def test_touching_cuts(self):
        self.assertEqual(subtract([(0, 5)], [(-2, 0), (5, 8)]), [(0, 5)])
        self.assertEqual(subtract([(0, 5)], [(1, 2), (2, 4)]), [(0, 1), (4, 5)])

    def test_cut_spans_multiple_sources(self):
        self.assertEqual(
            subtract([(0, 2), (4, 6), (8, 10)], [(1, 9)]),
            [(0, 1), (9, 10)],
        )

    def test_disjoint_and_empty_cuts(self):
        sources = [(4, 6), (0, 2)]
        for cuts in ([], [(-4, -2), (2, 4), (6, 8)]):
            self.assertEqual(subtract(sources, cuts), [(0, 2), (4, 6)])

    def test_invalid_bounds_all_functions(self):
        with self.assertRaises(ValueError):
            normalize([(2, 1)])
        with self.assertRaises(ValueError):
            subtract([(2, 1)], [])
        with self.assertRaises(ValueError):
            subtract([], [(2, 1)])
        with self.assertRaises(ValueError):
            contains([(0, 10), (2, 1)], 5)

    def test_no_mutation_or_aliasing(self):
        sources, cuts = [[5, 8], [0, 3]], [[1, 2]]
        original = copy.deepcopy((sources, cuts))
        normalized = normalize(sources)
        subtract(sources, cuts)
        contains(sources, 1)
        self.assertEqual((sources, cuts), original)
        sources[1][0] = -10
        self.assertEqual(normalized, [(0, 3), (5, 8)])

    def test_fractional_bounds_and_membership(self):
        self.assertEqual(subtract([(-1.5, 2.5)], [(0.5, 1.5)]),
                         [(-1.5, 0.5), (1.5, 2.5)])
        for point, expected in [(-1.5, True), (0.5, False), (1.5, True), (2.5, False)]:
            self.assertEqual(contains([(1.5, 2.5), (-1.5, 0.5)], point), expected)

    def test_iterators(self):
        self.assertEqual(normalize(iter([(3, 4), (1, 3)])), [(1, 4)])
        self.assertEqual(subtract(iter([(0, 4)]), iter([(1, 3)])), [(0, 1), (3, 4)])
        self.assertTrue(contains(iter([(0, 1)]), 0))

    def test_exhaustive_small_set_difference(self):
        # A discrete membership oracle checks every endpoint and midpoint.
        candidates = list(itertools.combinations_with_replacement(range(-1, 3), 2))
        sets = [[], *([x] for x in candidates), *map(list, itertools.combinations(candidates, 2))]
        points = [n / 2 for n in range(-3, 6)]
        for sources in sets:
            for cuts in sets:
                result = subtract(sources, cuts)
                self.assertTrue(all(a < b for a, b in result))
                self.assertTrue(all(left[1] < right[0] for left, right in zip(result, result[1:])))
                for point in points:
                    expected = (any(a <= point < b for a, b in sources)
                                and not any(a <= point < b for a, b in cuts))
                    self.assertEqual(contains(result, point), expected, (sources, cuts, point))


if __name__ == "__main__":
    unittest.main()
