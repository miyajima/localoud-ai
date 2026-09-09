import unittest
from intervals import normalize, subtract, contains
class Acceptance(unittest.TestCase):
 def test_merge(self): self.assertEqual(normalize([(5,8),(1,3),(3,5),(9,9)]),[(1,8)])
 def test_no_mutation(self):
  x=[(5,7),(1,3)]; normalize(x); self.assertEqual(x,[(5,7),(1,3)])
 def test_invalid(self):
  with self.assertRaises(ValueError): normalize([(3,2)])
 def test_subtract(self): self.assertEqual(subtract([(0,20)],[(3,5),(8,12),(15,25)]),[(0,3),(5,8),(12,15)])
 def test_overlap(self): self.assertEqual(subtract([(1,5),(3,9)],[(2,4),(3,6)]),[(1,2),(6,9)])
 def test_empty(self): self.assertEqual(subtract([],[(1,2)]),[])
 def test_edges(self):
  self.assertTrue(contains([(1,3)],1)); self.assertFalse(contains([(1,3)],3))
 def test_negative(self): self.assertEqual(normalize([(-5,-2),(-2,0)]),[(-5,0)])
 def test_full(self): self.assertEqual(subtract([(1,2)],[(0,3)]),[])
if __name__=='__main__': unittest.main()
