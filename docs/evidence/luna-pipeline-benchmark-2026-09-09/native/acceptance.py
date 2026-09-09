import unittest, copy
from scheduler import topological_order, execution_batches, critical_path_length

def t(deps=(),duration=1,group=None):return {'deps':list(deps),'duration':duration,'group':group}
class Acceptance(unittest.TestCase):
 def test_empty(self):
  self.assertEqual(topological_order({}),[]);self.assertEqual(execution_batches({},1),[]);self.assertEqual(critical_path_length({}),0)
 def test_new_ready_priority(self):
  self.assertEqual(topological_order({'a':t(),'b':t(['a']),'z':t()}),['a','b','z'])
 def test_batches_are_waves(self):
  self.assertEqual(execution_batches({'a':t(),'b':t(['a']),'z':t()},2),[['a','z'],['b']])
 def test_groups_skip_and_fill(self):
  tasks={'a':t(group='x'),'b':t(group='x'),'c':t(group='y'),'d':t()}
  self.assertEqual(execution_batches(tasks,3),[['a','c','d'],['b']])
 def test_capacity(self):
  self.assertEqual(execution_batches({x:t() for x in 'edcba'},2),[['a','b'],['c','d'],['e']])
 def test_longest_path(self):
  tasks={'a':t(duration=2),'b':t(['a'],3),'c':t(['a'],5),'d':t(['b','c'],4),'z':t(duration=20)}
  self.assertEqual(critical_path_length(tasks),20);tasks.pop('z');self.assertEqual(critical_path_length(tasks),11)
 def test_duplicate_dependencies(self):
  tasks={'a':t(),'b':t(['a','a'],2)}
  self.assertEqual(topological_order(tasks),['a','b']);self.assertEqual(execution_batches(tasks,2),[['a'],['b']]);self.assertEqual(critical_path_length(tasks),3)
 def test_input_immutable(self):
  tasks={'b':t(['a','a'],2,'x'),'a':t()};before=copy.deepcopy(tasks)
  topological_order(tasks);execution_batches(tasks,2);critical_path_length(tasks);self.assertEqual(tasks,before)
 def test_cycles(self):
  for tasks in [{'a':t(['a'])},{'a':t(['b']),'b':t(['a'])},{'a':t(),'b':t(['c']),'c':t(['b'])}]:
   for f in [topological_order,critical_path_length,lambda x:execution_batches(x,2)]:
    with self.assertRaises(ValueError):f(tasks)
 def test_unknown_dependency(self):
  for f in [topological_order,critical_path_length,lambda x:execution_batches(x,2)]:
   with self.assertRaises(ValueError):f({'a':t(['missing'])})
 def test_schema(self):
  bad=[None,[],{1:t()},{'':t()},{'a':{}},{'a':t(duration=True)},{'a':t(duration=0)},{'a':t(duration=1.5)},{'a':t(group='')},{'a':t(group=2)},{'a':dict(t(),extra=1)},{'a':dict(t(),deps='x')},{'a':dict(t(),deps=[[]])}]
  for tasks in bad:
   for f in [topological_order,critical_path_length,lambda x:execution_batches(x,2)]:
    with self.subTest(tasks=tasks),self.assertRaises(ValueError):f(tasks)
 def test_invalid_capacity(self):
  for n in [0,-1,True,1.5,None,'2']:
   with self.assertRaises(ValueError):execution_batches({},n)
if __name__=='__main__':unittest.main()
