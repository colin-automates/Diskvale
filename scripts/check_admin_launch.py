"""Verify the release UAC manifest and launch guards without opening a volume."""

import argparse
import ctypes
from ctypes import wintypes
import hashlib
import json
from pathlib import Path
import subprocess
import sys
import xml.etree.ElementTree as ET


def manifest_bytes(executable: Path) -> bytes:
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
    # Data/resource mapping only: no executable entry point or DllMain is run.
    module = kernel.LoadLibraryExW(str(executable), None, 0x2 | 0x20)
    if not module:
        raise ctypes.WinError(ctypes.get_last_error())
    try:
        resource = kernel.FindResourceW(module, ctypes.c_void_p(1), ctypes.c_void_p(24))
        if not resource:
            raise ctypes.WinError(ctypes.get_last_error())
        size = kernel.SizeofResource(module, resource)
        handle = kernel.LoadResource(module, resource)
        address = kernel.LockResource(handle)
        if not size or not address:
            raise ctypes.WinError(ctypes.get_last_error())
        return ctypes.string_at(address, size)
    finally:
        kernel.FreeLibrary(module)


def main():
    if sys.platform != "win32":
        raise SystemExit("This validation requires Windows.")
    project = Path(__file__).resolve().parents[1]
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--executable", type=Path, default=project / "target/release/canopy.exe")
    parser.add_argument("--debug-executable", type=Path, default=project / "target/debug/canopy.exe")
    parser.add_argument("--output", type=Path, default=project / "artifacts/launch-checks")
    args = parser.parse_args()
    executable = args.executable.resolve()
    debug_executable = args.debug_executable.resolve()
    evidence = args.output.resolve()
    evidence.mkdir(parents=True, exist_ok=True)
    raw_manifest = manifest_bytes(executable)
    levels = ET.fromstring(raw_manifest).findall(".//{*}requestedExecutionLevel")
    assert len(levels) == 1, "Expected exactly one requested execution level"
    assert levels[0].get("level") == "requireAdministrator", levels[0].attrib
    assert levels[0].get("uiAccess") == "false", levels[0].attrib
    (evidence / "release.manifest.xml").write_bytes(raw_manifest)
    shell = ctypes.WinDLL("shell32")
    shell.IsUserAnAdmin.argtypes = []
    shell.IsUserAnAdmin.restype = wintypes.BOOL
    administrator = bool(shell.IsUserAnAdmin())
    result = {
        "executable": str(executable),
        "sha256": hashlib.sha256(executable.read_bytes()).hexdigest(),
        "requested_execution_level": levels[0].attrib,
        "validation_token_administrator": administrator,
        "raw_volume_reads_performed": False,
    }
    if not administrator:
        # CreateProcess never answers UAC. Windows must reject this launch
        # before main; the folder target is also safe if the guard regresses.
        try:
            process = subprocess.Popen(
                [str(executable), "--scan", str(project)],
                stdout=subprocess.PIPE,
                stderr=subprocess.PIPE,
            )
        except OSError as error:
            assert error.winerror == 740, error
            result["release_create_process_error"] = error.winerror
        else:
            process.communicate(timeout=10)
            raise AssertionError("Release launched without administrator approval")
        debug_manifest = manifest_bytes(debug_executable)
        debug_levels = ET.fromstring(debug_manifest).findall(".//{*}requestedExecutionLevel")
        assert len(debug_levels) == 1
        assert debug_levels[0].get("level") == "asInvoker", debug_levels[0].attrib
        assert debug_levels[0].get("uiAccess") == "false", debug_levels[0].attrib
        debug = subprocess.run(
            [str(debug_executable), "--scan", str(project)],
            capture_output=True,
            text=True,
            timeout=10,
        )
        assert debug.returncode == 1, debug
        assert "requires administrator access" in debug.stderr, debug.stderr
        result["debug_unelevated_exit_code"] = debug.returncode
        result["debug_unelevated_error"] = debug.stderr.strip()
    else:
        result["unelevated_runtime_checks"] = "Skipped: validation token is elevated"
    (evidence / "launch-check.json").write_text(json.dumps(result, indent=2), encoding="utf-8")
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()
