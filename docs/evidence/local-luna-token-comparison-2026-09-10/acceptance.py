import copy
import importlib.util
import itertools
import random
import sys
import unittest

spec = importlib.util.spec_from_file_location('candidate', sys.argv[1])
m = importlib.util.module_from_spec(spec)
spec.loader.exec_module(m)
sys.argv = sys.argv[:1]

def points(xs):
    return {p for a,b in xs for p in range(a,b)}

def canonical(ps):
    out = []
    for p in sorted(ps):
        if out and out[-1][1] == p:
            out[-1] = (out[-1][0], p+1)
        else:
            out.append((p,p+1))
    return out

class Acceptance(unittest.TestCase):
    def test_examples_and_boundaries(self):
        self.assertEqual(m.normalize([(3,5),(1,3)]), [(1,5)])
        self.assertEqual(m.subtract([(0,10)],[(2,4),(6,20)]),[(0,2),(4,6)])
        self.assertEqual(m.earliest_slot([(1,3)],2,(0,5)),3)
        self.assertEqual(m.subtract([(0,2)],[(2,4),(-3,0)]),[(0,2)])
        self.assertEqual(m.earliest_slot([(-5,0),(5,9)],5,(0,5)),0)
        self.assertIsNone(m.earliest_slot([],6,(0,5)))

    def test_empty(self):
        self.assertEqual(m.normalize([]),[])
        self.assertEqual(m.subtract([],[]),[])
        self.assertEqual(m.earliest_slot([],1,(-2,2)),-2)

    def test_invalid_collections_and_entries(self):
        bad = [None, 1, 'x', {}, {1,2}, [(1,1)], [(2,1)], [(False,2)], [(0,True)], [(0,2.0)], [(0,)], [(0,1,2)], [None], ['12'], [(0,1), (4,3)]]
        for x in bad:
            with self.subTest(value=repr(x)):
                with self.assertRaises(ValueError): m.normalize(x)
                with self.assertRaises(ValueError): m.subtract(x, [])
                with self.assertRaises(ValueError): m.subtract([], x)
                with self.assertRaises(ValueError): m.earliest_slot(x, 99, (0,2))

    def test_invalid_slot_arguments(self):
        for d in [None, 0, -1, True, False, 1.0, '1', []]:
            with self.subTest(duration=repr(d)):
                with self.assertRaises(ValueError): m.earliest_slot([],d,(0,2))
        for w in [None, (), (1,), (1,2,3), (0,0), (2,0), (True,3), (0,2.0), '12', {0,2}]:
            with self.subTest(window=repr(w)):
                with self.assertRaises(ValueError): m.earliest_slot([],1,w)

    def test_seeded_oracle_and_immutability(self):
        rng = random.Random(91827)
        pool = list(itertools.combinations(range(-5,7),2))
        for i in range(250):
            xs = [list(rng.choice(pool)) for _ in range(rng.randrange(8))]
            ys = [list(rng.choice(pool)) for _ in range(rng.randrange(8))]
            before = copy.deepcopy((xs,ys))
            self.assertEqual(m.normalize(xs),canonical(points(xs)))
            self.assertEqual(m.subtract(xs,ys),canonical(points(xs)-points(ys)))
            a,b = rng.choice(pool)
            d = rng.randrange(1,15)
            expected = next((t for t in range(a,b-d+1) if not (set(range(t,t+d)) & points(xs))),None)
            self.assertEqual(m.earliest_slot(xs,d,(a,b)),expected)
            self.assertEqual((xs,ys),before)

    def test_tuple_input_and_large_integers(self):
        n = 10**30
        self.assertEqual(m.normalize(((n,n+2),(n+2,n+4))),[(n,n+4)])
        self.assertEqual(m.subtract(((n,n+10),),((n+2,n+8),)),[(n,n+2),(n+8,n+10)])
        self.assertEqual(m.earliest_slot(((n,n+2),),3,(n,n+5)),n+2)

unittest.main()
