#!/usr/bin/env python3
"""Portable harness checks. No subprocess, socket or Linux kernel operation runs.

    python -B scripts/wsl-secure-egress/tests/linux_static_test.py

These tests validate gating and evidence handling, not real firewall behavior.
"""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import socket
import struct
import subprocess
import sys
import unittest
from unittest.mock import Mock, patch


sys.dont_write_bytecode = True
HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("cor_linux_integration", HERE / "linux_integration.py")
harness = importlib.util.module_from_spec(spec)
# Import must never execute any OS command or open a network socket.
with patch.object(subprocess, "run", side_effect=AssertionError("command during import")), \
        patch.object(subprocess, "Popen", side_effect=AssertionError("process during import")), \
        patch.object(socket, "socket", side_effect=AssertionError("socket during import")):
    spec.loader.exec_module(harness)
manager_path = HERE.parent / "linux" / "manager.py"
manager_spec = importlib.util.spec_from_file_location("cor_egress_manager", manager_path)
manager = importlib.util.module_from_spec(manager_spec)
with patch.object(subprocess, "run", side_effect=AssertionError("command during import")):
    manager_spec.loader.exec_module(manager)


class GateTests(unittest.TestCase):
    def test_all_three_explicit_gates_are_required(self):
        for execute in (False, True):
            for ci in (None, "false", "TRUE", "1", "true"):
                for isolated in (None, "true", "0", "1"):
                    env = {key: value for key, value in
                           (("CI", ci), ("COR_EGRESS_ISOLATED_CI", isolated)) if value is not None}
                    expected = execute and ci == "true" and isolated == "1"
                    with self.subTest(execute=execute, ci=ci, isolated=isolated), \
                            patch.dict(os.environ, env, clear=True), \
                            patch.object(harness.sys, "platform", "linux"), \
                            patch.object(os, "geteuid", return_value=0, create=True), \
                            patch.object(subprocess, "run") as run:
                        if expected:
                            harness.execution_gate(execute)
                        else:
                            with self.assertRaises(harness.Failure):
                                harness.execution_gate(execute)
                        run.assert_not_called()

    def test_non_linux_and_non_root_refused(self):
        for platform, uid in (("win32", 0), ("linux", 1000)):
            with self.subTest(platform=platform, uid=uid), \
                    patch.dict(os.environ, {"CI": "true", "COR_EGRESS_ISOLATED_CI": "1"}, clear=True), \
                    patch.object(harness.sys, "platform", platform), \
                    patch.object(os, "geteuid", return_value=uid, create=True):
                with self.assertRaises(harness.Failure):
                    harness.execution_gate(True)

    def test_refused_entrypoint_has_no_side_effects(self):
        with patch.dict(os.environ, {}, clear=True), patch.object(harness, "command") as command, \
                patch.object(subprocess, "run") as run, patch.object(harness, "private_mounts") as mounts:
            with self.assertRaises(harness.Failure):
                harness.main(["--execute-isolated"])
            command.assert_not_called()
            run.assert_not_called()
            mounts.assert_not_called()

    def test_launcher_only_enters_unshare(self):
        with patch.object(harness, "execution_gate"), \
                patch.object(harness, "PAYLOAD_PYTHON", sys.executable), \
                patch.object(harness.shutil, "which", return_value="/usr/bin/tool"), \
                patch.object(harness, "namespace_ids", return_value={"net": 1, "mnt": 2, "pid": 3}), \
                patch.object(subprocess, "run", return_value=Mock(returncode=0)) as run, \
                patch.object(harness, "private_mounts") as mounts, \
                patch.object(harness, "Fixture") as fixture:
            self.assertEqual(harness.main(["--execute-isolated"]), 0)
            argv = run.call_args.args[0]
            self.assertEqual(argv[0], "unshare")
            for flag in ("--net", "--mount", "--pid", "--fork", "--mount-proc", "--kill-child=SIGKILL"):
                self.assertIn(flag, argv)
            self.assertIn("--_isolated-parent", argv)
            mounts.assert_not_called()
            fixture.assert_not_called()
            run.assert_called_once()

    def test_internal_child_cannot_use_launcher_namespaces(self):
        original = {"net": 1, "mnt": 2, "pid": 3}
        for current in (original, {"net": 4, "mnt": 2, "pid": 5}):
            with patch.object(os, "getpid", return_value=1), \
                    patch.object(harness, "namespace_ids", return_value=current), \
                    patch.object(harness, "command") as command:
                with self.assertRaises(harness.Failure):
                    harness.verify_child(original)
                command.assert_not_called()
        with patch.object(os, "getpid", return_value=123):
            with self.assertRaises(harness.Failure):
                harness.verify_child(original)


class EvidenceTests(unittest.TestCase):
    def fixture(self):
        fixture = object.__new__(harness.Fixture)
        fixture.evidence = []
        fixture.control = Mock()
        fixture.observe = Mock(return_value=dict(direct=0, tunnel=0, wireguard=0))
        fixture.probe = Mock(return_value=dict(nonce="fake", sent=True, reply=False))
        fixture.receipts = Mock(return_value=[])
        return fixture

    def test_send_success_or_timeout_is_not_a_negative_proof(self):
        fixture = self.fixture()
        fixture.deny_counters = Mock(return_value=7)
        with patch.object(harness.time, "sleep"):
            with self.assertRaisesRegex(harness.Failure, "deny counter"):
                fixture.negative("send-is-not-delivery")

    def test_nonce_receipt_defeats_negative_even_with_drop_counter(self):
        fixture = self.fixture()
        fixture.deny_counters = Mock(side_effect=[1, 2])
        fixture.receipts.return_value = [dict(nonce="fake")]
        with patch.object(harness.time, "sleep"):
            with self.assertRaisesRegex(harness.Failure, "reached receiver"):
                fixture.negative("received-is-a-leak")

    def test_escaped_packet_defeats_negative_without_application_receipt(self):
        fixture = self.fixture()
        fixture.deny_counters = Mock(side_effect=[1, 2])
        fixture.observe.side_effect = [dict(direct=0), dict(direct=1)]
        with patch.object(harness.time, "sleep"):
            with self.assertRaisesRegex(harness.Failure, "escaped"):
                fixture.negative("escaped-syn-is-a-leak")

    def test_counter_and_receiver_controls_required_for_negative(self):
        fixture = self.fixture()
        fixture.deny_counters = Mock(side_effect=[1, 2])
        with patch.object(harness.time, "sleep"):
            fixture.negative("bounded-refusal")
        self.assertEqual(fixture.control.call_count, 2)
        self.assertEqual(fixture.evidence[0]["received"], 0)

    def test_echo_without_receipt_cannot_pass_positive(self):
        fixture = self.fixture()
        fixture.probe.return_value = dict(nonce="fake", reply=True)
        fixture.observe.side_effect = [dict(direct=0), dict(direct=1)]
        with self.assertRaisesRegex(harness.Failure, "receipt"):
            fixture.positive("no-receiver-record")

    def test_wrong_tunnel_source_cannot_pass_positive(self):
        fixture = self.fixture()
        fixture.probe.return_value = dict(nonce="fake", reply=True)
        fixture.receipts.return_value = [dict(nonce="fake", kind="tcp", source="192.0.2.1")]
        fixture.observe.side_effect = [dict(direct=0, tunnel=0, wireguard=0),
                                       dict(direct=0, tunnel=1, wireguard=1)]
        with self.assertRaisesRegex(harness.Failure, "wrong tunnel source"):
            fixture.positive("wrong-source", via_tunnel=True)

    def test_inline_scripts_and_owned_files_parse_without_execution(self):
        for name in ("RECEIVER", "CLIENT", "SANDBOX_AUDIT"):
            compile(getattr(harness, name), f"<{name}>", "exec")
        for path in (HERE / "linux_integration.py", HERE / "linux_static_test.py"):
            compile(path.read_text(encoding="utf-8"), str(path), "exec")

    def test_transport_pmtu_policy_rule_uses_supported_selectors(self):
        manager_path = HERE.parent / "linux" / "manager.py"
        source = manager_path.read_text(encoding="utf-8")
        self.assertIn("'to', TRANSPORT_IP + '/32', 'lookup', 'main'", source)
        self.assertNotIn("'protocol', 'icmp'", source)
        self.assertIn('icmp type 3 icmp code 4 ct state related counter accept', source)
        self.assertIn('PREF_TRANSPORT_PMTU = 10072', source)
        self.assertIn('PREF_DEFAULT_DENY = 10074', source)

    def test_sandbox_seccomp_denies_namespace_entry_and_creation(self):
        program = manager.seccomp_filter()
        instructions = [struct.unpack_from("HBBI", program, offset)
                        for offset in range(0, len(program), 8)]
        self.assertTrue(instructions)
        self.assertTrue(all(len(instruction) == 4 for instruction in instructions))

        def evaluate(arch, syscall, arg0=0):
            data = bytearray(32)
            struct.pack_into("I", data, manager.SECCOMP_ARCH_OFFSET, arch)
            struct.pack_into("I", data, manager.SECCOMP_NR_OFFSET, syscall)
            struct.pack_into("I", data, manager.SECCOMP_ARG0_OFFSET, arg0)
            accumulator = 0
            pc = 0
            while pc < len(instructions):
                code, jump_true, jump_false, value = instructions[pc]
                if code == 0x20:  # BPF_LD | BPF_W | BPF_ABS
                    accumulator = struct.unpack_from("I", data, value)[0]
                    pc += 1
                elif code == 0x15:  # BPF_JMP | BPF_JEQ | BPF_K
                    pc += (jump_true if accumulator == value else jump_false) + 1
                elif code == 0x45:  # BPF_JMP | BPF_JSET | BPF_K
                    pc += (jump_true if accumulator & value else jump_false) + 1
                elif code == 0x06:  # BPF_RET | BPF_K
                    return value
                else:
                    self.fail(f"unexpected seccomp opcode 0x{code:x}")
            self.fail("seccomp program fell through")

        arch_name = manager.platform.machine().lower()
        arch = manager.AUDIT_ARCHES[arch_name]
        syscalls = manager.SYSCALLS[arch_name]
        denied = manager.SECCOMP_RET_ERRNO | manager.EPERM
        self.assertEqual(evaluate(arch, syscalls["unshare"]), denied)
        self.assertEqual(evaluate(arch, syscalls["setns"]), denied)
        self.assertEqual(evaluate(arch, syscalls["clone3"]),
                         manager.SECCOMP_RET_ERRNO | manager.ENOSYS)
        self.assertEqual(evaluate(arch, syscalls["clone"]), manager.SECCOMP_RET_ALLOW)
        self.assertEqual(
            evaluate(arch, syscalls["clone"], manager.CLONE_NAMESPACE_FLAGS & -manager.CLONE_NAMESPACE_FLAGS),
            denied,
        )
        self.assertEqual(evaluate(arch, syscalls["clone"], 0x00000100), manager.SECCOMP_RET_ALLOW)
        source = manager_path.read_text(encoding="utf-8")
        self.assertIn("'unshare'", source)
        self.assertIn("'setns'", source)
        self.assertIn("'clone3'", source)
        self.assertIn("'--seccomp'", source)

    def test_sandbox_seccomp_refuses_unknown_architecture(self):
        with patch.object(manager.platform, "machine", return_value="unknown-cpu"):
            with self.assertRaisesRegex(manager.Blocked, "Unsupported architecture"):
                manager.seccomp_filter()

    def test_nonce_generator_has_unique_dns_safe_labels(self):
        tokens = {harness.nonce() for _ in range(100)}
        self.assertEqual(len(tokens), 100)
        self.assertTrue(all(len(token) == 32 and set(token) <= set("0123456789abcdef") for token in tokens))

    def test_udp_client_rejects_wrong_nonce_and_wrong_sender(self):
        for payload, peer, expected in ((b"aa" * 16, (harness.TARGET, 18080), True),
                                        (b"bb" * 16, (harness.TARGET, 18080), False),
                                        (b"aa" * 16, (harness.ENDPOINT, 18080), False)):
            fake_socket = Mock()
            fake_socket.recvfrom.return_value = (payload, peer)
            config = dict(mode="udp", host=harness.TARGET, port=18080, nonce="aa" * 16)
            output = io.StringIO()
            with self.subTest(payload=payload, peer=peer), \
                    patch.object(socket, "socket", return_value=fake_socket), \
                    patch.object(sys, "argv", ["client", json.dumps(config)]), contextlib.redirect_stdout(output):
                exec(compile(harness.CLIENT, "<CLIENT>", "exec"), {})
            result = json.loads(output.getvalue())
            self.assertEqual(result["reply"], expected)
            self.assertTrue(result["sent"])


if __name__ == "__main__":
    unittest.main(verbosity=2)
