"""Regression boundaries for preserving executable bytes during signing."""
import importlib.util
from pathlib import Path
import struct
import unittest

spec = importlib.util.spec_from_file_location('signed_content', Path(__file__).parents[1] / 'check_signed_content.py')
module = importlib.util.module_from_spec(spec)
spec.loader.exec_module(module)


def unsigned_executable():
    data = bytearray(513)
    data[:2] = b'MZ'
    struct.pack_into('<I', data, 60, 128)
    data[128:132] = b'PE\0\0'
    struct.pack_into('<H', data, 132, 0x8664)
    struct.pack_into('<H', data, 148, 240)
    struct.pack_into('<H', data, 152, 0x20B)
    struct.pack_into('<I', data, 260, 16)
    data[512] = 42
    return bytes(data)


def signed_executable(original):
    data = bytearray(original)
    checksum, directory = module.pe_fields(original)
    data.extend(bytes((-len(data)) % 8))
    offset = len(data)
    certificate = struct.pack('<IHH', 12, 0x200, 2) + b'test' + bytes(4)
    data.extend(certificate)
    struct.pack_into('<I', data, checksum, 1234)
    struct.pack_into('<II', data, directory, offset, len(certificate))
    return data


class ContentTests(unittest.TestCase):
    def setUp(self):
        self.original = unsigned_executable()
        self.signed = signed_executable(self.original)

    def test_appended_certificate_and_required_header_changes_are_allowed(self):
        module.verify_content(self.original, self.signed)

    def test_modified_executable_and_overlay_bytes_are_rejected(self):
        for offset in (100, 400, 512):
            with self.subTest(offset=offset):
                altered = bytearray(self.signed)
                altered[offset] ^= 1
                with self.assertRaises(ValueError):
                    module.verify_content(self.original, altered)

    def test_nonzero_alignment_padding_is_rejected(self):
        self.signed[513] = 1
        with self.assertRaises(ValueError):
            module.verify_content(self.original, self.signed)

    def test_certificate_bounds_and_extra_trailing_bytes_are_rejected(self):
        _, directory = module.pe_fields(self.original)
        for offset, length in ((512, 16), (520, 15), (520, 17), (521, 15)):
            with self.subTest(offset=offset, length=length):
                altered = bytearray(self.signed)
                struct.pack_into('<II', altered, directory, offset, length)
                with self.assertRaises(ValueError):
                    module.verify_content(self.original, altered)
        with self.assertRaises(ValueError):
            module.verify_content(self.original, self.signed + b'junk')

    def test_already_signed_input_is_rejected(self):
        with self.assertRaises(ValueError):
            module.verify_content(self.signed, self.signed)

    def test_missing_signature_and_malformed_certificate_are_rejected(self):
        with self.assertRaises(ValueError):
            module.verify_content(self.original, self.original)
        for revision, kind, length in ((0x100, 2, 12), (0x200, 1, 12), (0x200, 2, 7), (0x200, 2, 100)):
            with self.subTest(revision=revision, kind=kind, length=length):
                altered = bytearray(self.signed)
                struct.pack_into('<IHH', altered, 520, length, revision, kind)
                with self.assertRaises(ValueError):
                    module.verify_content(self.original, altered)

    def test_truncated_and_wrong_architecture_pe_are_rejected(self):
        for data in (b'MZ', self.original[:151], self.original[:250]):
            with self.subTest(length=len(data)):
                with self.assertRaises(ValueError):
                    module.pe_fields(data)
        altered = bytearray(self.original)
        struct.pack_into('<H', altered, 132, 0x14C)
        with self.assertRaises(ValueError):
            module.pe_fields(altered)


if __name__ == '__main__':
    unittest.main()
