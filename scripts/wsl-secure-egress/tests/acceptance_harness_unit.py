"""Offline validation for live-acceptance endpoint selection.

python -B scripts/wsl-secure-egress/tests/acceptance_harness_unit.py
"""
import importlib.util
from pathlib import Path
import unittest


SOURCE = Path(__file__).with_name('failclosed_acceptance.py')
SPEC = importlib.util.spec_from_file_location('cor_failclosed_acceptance', SOURCE)
acceptance = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(acceptance)


class EndpointValidationTests(unittest.TestCase):
    def test_accepts_public_numeric_ipv4(self):
        self.assertEqual(
            acceptance.validate_endpoint_ipv4('8.8.8.8'),
            '8.8.8.8',
        )

    def test_rejects_missing_or_non_numeric_endpoint(self):
        for value in ('', 'vps.example.com', '2001:db8::1', 'not-an-address'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                acceptance.validate_endpoint_ipv4(value)

    def test_rejects_non_public_ipv4(self):
        for value in ('127.0.0.1', '10.0.0.1', '192.168.1.1'):
            with self.subTest(value=value), self.assertRaises(ValueError):
                acceptance.validate_endpoint_ipv4(value)


if __name__ == '__main__':
    unittest.main()
