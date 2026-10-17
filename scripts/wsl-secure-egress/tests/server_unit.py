"""Offline validation; importing the VPS module must never change networking."""
import base64
import importlib.util
from pathlib import Path
import unittest
from unittest.mock import patch

SOURCE = Path(__file__).resolve().parents[1]/'server/manage.py'
spec = importlib.util.spec_from_file_location('cor_vps', SOURCE)
server = importlib.util.module_from_spec(spec)
spec.loader.exec_module(server)


class ProfileTests(unittest.TestCase):
    def profile(self):
        return dict(schema=1, server_address='10.77.0.1/24', peer_address='10.77.0.2',
                    resolver_upstream='192.0.2.53', listen_port=51820,
                    peer_public_key=base64.b64encode(bytes(range(32))).decode())

    def test_validation_is_offline(self):
        with patch.object(server, 'run', side_effect=AssertionError('Unexpected process')):
            address, peer, resolver = server.validate(self.profile())
        self.assertEqual(str(peer), '10.77.0.2')
        self.assertNotEqual(peer, address.ip)
        self.assertEqual(str(resolver), '192.0.2.53')

    def test_rejects_wrong_peer_address(self):
        for peer in ('10.77.0.1', '10.78.0.2', '::1'):
            profile = self.profile()
            profile['peer_address'] = peer
            with self.subTest(peer=peer), self.assertRaises(ValueError):
                server.validate(profile)

    def test_rejects_bad_key(self):
        for key in ('', 'not-a-key', base64.b64encode(b'short').decode()):
            profile = self.profile()
            profile['peer_public_key'] = key
            with self.subTest(key=key), self.assertRaises(ValueError):
                server.validate(profile)

    def test_rejects_non_unicast_resolver_or_invalid_port(self):
        for field, value in [('resolver_upstream','127.0.0.1'), ('resolver_upstream','224.0.0.1'),
                             ('listen_port',0), ('listen_port',65536), ('listen_port',True)]:
            profile = self.profile()
            profile[field] = value
            with self.subTest(field=field, value=value), self.assertRaises(ValueError):
                server.validate(profile)


if __name__ == '__main__':
    unittest.main()
