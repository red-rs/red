import unittest

def add_numbers(a, b):
    return a + b


class TestAddNumbers(unittest.TestCase):

    def test_add_positive_numbers(self):
        print("test_add_positive_numbers")
        result = add_numbers(1, 7)
        self.assertEqual(result, 8)

    def test_add_negative_numbers(self):
        result = add_numbers(-2, -3)
        self.assertEqual(result, -5)


    def test_add_fail(self):
        result = add_numbers(0, 7)
        self.assertEqual(result, 8)


