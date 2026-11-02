#!/usr/bin/env python3
"""Tests for tools/check-glibc.py: `python3 tools/test-check-glibc.py` (stdlib only).

Builds small ELF64 files with a `.gnu.version_r` section so the weak/required logic and the damaged
file handling are checked without a real binary: the parser must answer with its documented errors
(ValueError, struct.error) and never with an exception main() does not catch.
"""
import importlib.util
import os
import struct
import subprocess
import sys
import tempfile
import unittest

HERE = os.path.dirname(os.path.abspath(__file__))
SCRIPT = os.path.join(HERE, "check-glibc.py")
spec = importlib.util.spec_from_file_location("check_glibc", SCRIPT)
check = importlib.util.module_from_spec(spec)
spec.loader.exec_module(check)

SHT_STRTAB = 3
SHT_GNU_VERNEED = 0x6FFFFFFE


def make_elf(versions, library=b"libc.so.6"):
    """An ELF64 LE image whose only version need lists `versions`: [(\"GLIBC_2.35\", weak?), ...]."""
    strtab = b"\0" + library + b"\0"
    name_offsets = []
    for name, _ in versions:
        name_offsets.append(len(strtab))
        strtab += name.encode() + b"\0"
    vern = struct.pack("<HHIII", 1, len(versions), 1, 16, 0)
    for index, (name, weak) in enumerate(versions):
        last = index == len(versions) - 1
        vern += struct.pack("<IHHII", 0, 2 if weak else 0, 0, name_offsets[index], 0 if last else 16)
    header_size, section_size = 64, 64
    vern_offset = header_size
    str_offset = vern_offset + len(vern)
    shoff = str_offset + len(strtab)
    header = bytearray(header_size)
    header[:4] = b"\x7fELF"
    header[4], header[5] = 2, 1
    struct.pack_into("<Q", header, 0x28, shoff)
    struct.pack_into("<HHH", header, 0x3A, section_size, 3, 2)

    def section(kind, offset, size, link=0, info=0):
        return struct.pack("<IIQQQQIIQQ", 0, kind, 0, 0, offset, size, link, info, 1, 0)

    sections = (
        section(0, 0, 0)
        + section(SHT_GNU_VERNEED, vern_offset, len(vern), link=2, info=1)
        + section(SHT_STRTAB, str_offset, len(strtab))
    )
    return bytes(header) + vern + strtab + sections


class RequiredVersusWeak(unittest.TestCase):
    def test_the_highest_required_and_weak_versions_are_reported_apart(self):
        data = make_elf([("GLIBC_2.9", False), ("GLIBC_2.35", False), ("GLIBC_2.39", True)])
        self.assertEqual(check.glibc_needs(data), ("2.35", "2.39"))

    def test_versions_compare_by_number_not_by_text(self):
        self.assertEqual(check.glibc_needs(make_elf([("GLIBC_2.9", False), ("GLIBC_2.35", False)]))[0], "2.35")

    def test_other_version_tags_are_ignored(self):
        self.assertEqual(check.glibc_needs(make_elf([("GLIBC_PRIVATE", False), ("GLIBCXX_3.4", False)])), (None, None))

    def test_only_weak_versions_leave_nothing_required(self):
        self.assertEqual(check.glibc_needs(make_elf([("GLIBC_2.39", True)])), (None, "2.39"))


class DamagedFiles(unittest.TestCase):
    """Every damaged input is a ValueError or struct.error (what main() reports), never anything else."""

    CAUGHT = (ValueError, struct.error)

    def assertCaught(self, data):
        try:
            check.glibc_needs(data)
        except self.CAUGHT:
            pass  # the documented outcome

    def test_every_truncation_of_a_valid_file_is_reported_not_raised(self):
        data = make_elf([("GLIBC_2.35", False), ("GLIBC_2.39", True)])
        for cut in range(len(data)):
            with self.subTest(cut=cut):
                self.assertCaught(data[:cut])

    def test_tiny_elf_prefixes_are_refused(self):
        for size in range(0, 64):
            with self.subTest(size=size), self.assertRaises(ValueError):
                check.glibc_needs((b"\x7fELF\x02\x01" + bytes(64))[:size])

    def test_a_version_name_with_no_terminator_is_refused(self):
        data = bytearray(make_elf([("GLIBC_2.35", False)]))
        end = bytes(data).index(b"GLIBC_2.35") + len("GLIBC_2.35")
        data[end] = ord("X")  # the NUL after the name
        # Pad so a scan for the terminator runs off the end of the file.
        self.assertCaught(bytes(data[: bytes(data).index(b"GLIBC_2.35") + 11]))

    def test_invalid_utf8_in_a_name_is_refused(self):
        data = bytearray(make_elf([("GLIBC_2.35", False)]))
        start = bytes(data).index(b"GLIBC_2.35")
        data[start : start + 3] = b"\xff\xfe\xfd"
        self.assertCaught(bytes(data))

    def test_a_name_offset_outside_the_string_table_is_refused(self):
        data = bytearray(make_elf([("GLIBC_2.35", False)]))
        vna_name_at = 64 + 16 + 8  # file header, Verneed entry, then vna_hash/flags/other
        struct.pack_into("<I", data, vna_name_at, 0xFFFFFF00)
        self.assertCaught(bytes(data))

    def test_a_section_table_outside_the_file_is_refused(self):
        data = bytearray(make_elf([("GLIBC_2.35", False)]))
        struct.pack_into("<Q", data, 0x28, 0xFFFFFFFFFFFF)
        self.assertCaught(bytes(data))


class Command(unittest.TestCase):
    def run_check(self, data, *extra):
        with tempfile.NamedTemporaryFile(delete=False) as file:
            file.write(data)
        try:
            return subprocess.run(
                [sys.executable, SCRIPT, file.name, *extra], capture_output=True, text=True
            )
        finally:
            os.unlink(file.name)

    def test_a_damaged_file_exits_1_with_a_message_and_no_traceback(self):
        for data in (b"\x7fELF", b"\x7fELF\x02", b"\x7fELF\x02\x01", b"", b"not an elf at all"):
            with self.subTest(size=len(data)):
                result = self.run_check(data)
                self.assertEqual(result.returncode, 1, result.stderr)
                self.assertIn("error:", result.stderr)
                self.assertNotIn("Traceback", result.stderr)

    def test_the_baseline_decides_the_exit_status(self):
        data = make_elf([("GLIBC_2.35", False), ("GLIBC_2.39", True)])
        self.assertEqual(self.run_check(data).returncode, 0)  # default baseline 2.35: weak 2.39 is fine
        self.assertEqual(self.run_check(data, "2.34").returncode, 1)
        self.assertEqual(self.run_check(make_elf([("GLIBC_2.39", False)])).returncode, 1)


if __name__ == "__main__":
    unittest.main()
