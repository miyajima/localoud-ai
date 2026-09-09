import importlib.util
import pathlib
import sys
spec = importlib.util.spec_from_file_location("candidate", pathlib.Path(sys.argv[1]) / "slug.py")
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)
cases = [("Hello, World!", "hello-world"), (" A___B ", "a-b"), ("", ""), ("日本ABC", "abc"), ("a--b", "a-b"), ("123", "123")]
for value, expected in cases:
    assert module.slugify(value) == expected, (value, expected)
print("6 fixed cases passed")
