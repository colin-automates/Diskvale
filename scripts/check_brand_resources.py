"""Read Diskvale's Windows branding resources without executing the application."""
import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
import re
from pathlib import Path
import struct
import sys
import xml.etree.ElementTree as ET


def main():
    if sys.platform != "win32":
        raise SystemExit("Windows resource verification requires Windows.")
    root = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, default=root / "target/x86_64-pc-windows-msvc/release/canopy.exe")
    parser.add_argument("--output", type=Path, default=root / "artifacts/resource-checks")
    args = parser.parse_args()
    executable = args.executable.resolve()
    output = args.output.resolve()
    output.mkdir(parents=True, exist_ok=True)
    # Inspect the PE import table, rather than relying on compiler flags, to
    # verify that portable releases do not need a separate Visual C++ runtime.
    data = executable.read_bytes()
    pe = struct.unpack_from("<I", data, 60)[0]
    section_count = struct.unpack_from("<H", data, pe + 6)[0]
    optional_size = struct.unpack_from("<H", data, pe + 20)[0]
    optional = pe + 24
    assert struct.unpack_from("<H", data, optional)[0] == 0x20B, "Expected a Windows x64 PE"
    import_rva, _ = struct.unpack_from("<II", data, optional + 112 + 8)
    sections = [struct.unpack_from("<IIII", data, optional + optional_size + i * 40 + 8) for i in range(section_count)]

    def file_offset(rva):
        return next(raw + (rva - address) for length, address, raw_size, raw in sections if address <= rva < address + max(length, raw_size))

    imports = []
    position = file_offset(import_rva)
    while any(struct.unpack_from("<IIIII", data, position)):
        start = file_offset(struct.unpack_from("<I", data, position + 12)[0])
        imports.append(data[start:data.index(b"\0", start)].decode("ascii"))
        position += 20
    assert not any(name.upper().startswith(("VCRUNTIME", "MSVCP")) for name in imports), imports
    kernel = ctypes.WinDLL("kernel32", use_last_error=True)
    kernel.LoadLibraryExW.argtypes = [wintypes.LPCWSTR, wintypes.HANDLE, wintypes.DWORD]
    kernel.LoadLibraryExW.restype = wintypes.HMODULE
    kernel.FindResourceW.argtypes = [wintypes.HMODULE, ctypes.c_void_p, ctypes.c_void_p]
    kernel.FindResourceW.restype = wintypes.HRSRC
    kernel.SizeofResource.argtypes = [wintypes.HMODULE, wintypes.HRSRC]
    kernel.SizeofResource.restype = wintypes.DWORD
    kernel.LoadResource.argtypes = [wintypes.HMODULE, wintypes.HRSRC]
    kernel.LoadResource.restype = wintypes.HGLOBAL
    kernel.LockResource.argtypes = [wintypes.HGLOBAL]
    kernel.LockResource.restype = ctypes.c_void_p
    kernel.FreeLibrary.argtypes = [wintypes.HMODULE]
    kernel.FreeLibrary.restype = wintypes.BOOL
    module = kernel.LoadLibraryExW(str(executable), None, 0x2 | 0x20)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())

    def resource(kind, name=1):
        entry = kernel.FindResourceW(module, ctypes.c_void_p(name), ctypes.c_void_p(kind))
        if not entry:
            raise ctypes.WinError(ctypes.get_last_error())
        size = kernel.SizeofResource(module, entry)
        address = kernel.LockResource(kernel.LoadResource(module, entry))
        if not address or not size:
            raise ctypes.WinError(ctypes.get_last_error())
        return ctypes.string_at(address, size)

    try:
        group = resource(14)
        reserved, kind, count = struct.unpack_from("<HHH", group)
        assert (reserved, kind, count) == (0, 1, 10)
        frames = []
        for index in range(count):
            width, height, colors, _, planes, bits, size, identifier = struct.unpack_from("<BBBBHHIH", group, 6 + 14 * index)
            width, height = width or 256, height or 256
            assert width == height and planes == 1 and bits == 32 and colors == 0
            payload = resource(3, identifier)
            assert len(payload) == size
            expected = root / f"assets/brand/diskvale-app-{width}.png"
            assert payload == expected.read_bytes(), f"Embedded {width}px icon differs from the brand artwork"
            (output / f"embedded-icon-{width}.png").write_bytes(payload)
            frames.append(width)
        assert frames == [16, 20, 24, 32, 40, 48, 64, 96, 128, 256]
        version = resource(16)
        levels = ET.fromstring(resource(24)).findall(".//{*}requestedExecutionLevel")
        assert len(levels) == 1
        assert levels[0].get("level") == "requireAdministrator"
        assert levels[0].get("uiAccess") == "false"
    finally:
        kernel.FreeLibrary(module)

    version_api = ctypes.WinDLL("version", use_last_error=True)
    version_api.VerQueryValueW.argtypes = [ctypes.c_void_p, wintypes.LPCWSTR, ctypes.POINTER(ctypes.c_void_p), ctypes.POINTER(wintypes.UINT)]
    version_api.VerQueryValueW.restype = wintypes.BOOL
    block = ctypes.create_string_buffer(version)
    values = {}
    for field in ["ProductName", "FileDescription", "InternalName", "OriginalFilename", "ProductVersion", "FileVersion"]:
        address = ctypes.c_void_p()
        length = wintypes.UINT()
        assert version_api.VerQueryValueW(block, f"\\StringFileInfo\\040904B0\\{field}", ctypes.byref(address), ctypes.byref(length))
        values[field] = ctypes.wstring_at(address, length.value - 1)
    assert values["ProductName"] == values["InternalName"] == "Diskvale"
    assert values["OriginalFilename"] == "Diskvale.exe"
    assert values["FileDescription"] == "Diskvale - Disk Explorer"
    expected_version = re.search(r'^version\s*=\s*"([^"]+)"', (root / "Cargo.toml").read_text(encoding="utf-8"), re.M).group(1)
    assert values["ProductVersion"] == values["FileVersion"] == expected_version

    # Ask Windows to extract the large/small icons used by shell surfaces.
    shell = ctypes.WinDLL("shell32", use_last_error=True)
    shell.ExtractIconExW.argtypes = [wintypes.LPCWSTR, ctypes.c_int, ctypes.POINTER(wintypes.HICON), ctypes.POINTER(wintypes.HICON), wintypes.UINT]
    shell.ExtractIconExW.restype = wintypes.UINT
    user = ctypes.WinDLL("user32", use_last_error=True)
    user.DestroyIcon.argtypes = [wintypes.HICON]
    user.DestroyIcon.restype = wintypes.BOOL
    large, small = wintypes.HICON(), wintypes.HICON()
    extracted = shell.ExtractIconExW(str(executable), 0, ctypes.byref(large), ctypes.byref(small), 1)
    try:
        assert extracted == 2 and large.value and small.value, "Windows could not extract both app icons"
    finally:
        if large.value:
            user.DestroyIcon(large)
        if small.value:
            user.DestroyIcon(small)
    result = {
        "executable": str(executable),
        "sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "embedded_icon_sizes": frames,
        "embedded_icons_match_source": True,
        "windows_large_and_small_icon_extraction": "passed",
        "windows_icons_extracted": extracted,
        "version_info": values,
        "requested_execution_level": levels[0].attrib,
        "imported_windows_libraries": sorted(imports),
        "separate_visual_cpp_runtime_required": False,
        "application_executed": False,
        "raw_volume_reads_performed": False,
    }
    (output / "branding-check.json").write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
