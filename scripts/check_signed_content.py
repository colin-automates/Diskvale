"""Ensure Authenticode added a certificate without changing the built PE bytes.

Windows trust/signature verification is performed separately by PowerShell.
This check permits only the PE checksum, certificate directory, and an aligned
certificate appended to the original file to differ.
"""
import argparse
from pathlib import Path
import struct


def pe_fields(data):
    if len(data) < 64 or data[:2] != b'MZ':
        raise ValueError('Missing DOS header')
    pe = struct.unpack_from('<I', data, 60)[0]
    if pe + 24 > len(data) or data[pe:pe + 4] != b'PE\0\0':
        raise ValueError('Missing PE header')
    if struct.unpack_from('<H', data, pe + 4)[0] != 0x8664:
        raise ValueError('Expected a Windows x64 executable')
    optional = pe + 24
    optional_size = struct.unpack_from('<H', data, pe + 20)[0]
    if optional_size < 152 or optional + optional_size > len(data):
        raise ValueError('Truncated optional header')
    if struct.unpack_from('<H', data, optional)[0] != 0x20B:
        raise ValueError('Expected a PE32+ executable')
    if struct.unpack_from('<I', data, optional + 108)[0] < 5:
        raise ValueError('Missing certificate directory')
    return optional + 64, optional + 112 + 4 * 8


def verify_content(unsigned, signed):
    checksum, directory = pe_fields(unsigned)
    if pe_fields(signed) != (checksum, directory):
        raise ValueError('PE layout changed during signing')
    if any(struct.unpack_from('<II', unsigned, directory)):
        raise ValueError('Signing input already has a certificate directory')
    offset, size = struct.unpack_from('<II', signed, directory)
    if offset < len(unsigned) or offset % 8 or size < 8 or offset + size != len(signed):
        raise ValueError('Invalid appended certificate bounds')
    if offset - len(unsigned) > 7 or any(signed[len(unsigned):offset]):
        raise ValueError('Unexpected bytes before the appended certificate')
    before = bytearray(unsigned)
    after = bytearray(signed[:len(unsigned)])
    for start, length in ((checksum, 4), (directory, 8)):
        before[start:start + length] = after[start:start + length] = bytes(length)
    if before != after:
        raise ValueError('Executable bytes changed beyond Authenticode fields')
    position = offset
    while position < len(signed):
        if position + 8 > len(signed):
            raise ValueError('Truncated certificate header')
        length, revision, kind = struct.unpack_from('<IHH', signed, position)
        if length < 8 or revision != 0x200 or kind != 2 or position + length > len(signed):
            raise ValueError('Invalid WIN_CERTIFICATE entry')
        end = position + ((length + 7) // 8) * 8
        if end > len(signed) or any(signed[position + length:end]):
            raise ValueError('Invalid certificate alignment padding')
        position = end


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('unsigned', type=Path)
    parser.add_argument('signed', type=Path)
    args = parser.parse_args()
    verify_content(args.unsigned.read_bytes(), args.signed.read_bytes())
    print('Only Authenticode fields and the appended certificate changed.')
