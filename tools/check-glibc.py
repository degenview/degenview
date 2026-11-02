#!/usr/bin/env python3
"""Fails when a Linux binary needs a newer glibc than the baseline we promise.

    tools/check-glibc.py <binary> [max-version]     (max defaults to 2.35, Ubuntu 22.04's glibc)

A binary records the glibc symbol versions it was linked against (the ELF `.gnu.version_r`
section), so building on a newer distro than the oldest one we mean to support silently raises the
floor. The rc.11 arm64 tarball, built on Ubuntu 24.04, needs GLIBC_2.39 and does not start on
Ubuntu 22.04. On a 22.04 builder the link itself already refuses anything newer, so this guards
the promise against the build host drifting (someone writing `ubuntu-latest` again) and makes the
baseline something CI states rather than something we hope for.

Only versions the loader insists on count. A version every reference to which is weak
(VER_FLG_WEAK, as Rust's std makes `pidfd_spawnp` on x86-64) is reported but does not fail: the
loader starts the binary without it. `strings | grep GLIBC_` cannot tell the two apart and would
have called the rc.11 x64 tarball too new for Ubuntu 22.04.

Standard library only, ELF64 little-endian (x86-64 and aarch64), so it runs on a developer's Mac too.
"""
import struct
import sys

SHT_GNU_VERNEED = 0x6FFFFFFE
VER_FLG_WEAK = 0x2


def version_key(version):
    return tuple(int(part) for part in version.split("."))


def glibc_needs(data):
    """(required, weak): the highest GLIBC_x.y per kind, as strings, or None."""
    # 64 bytes is the ELF64 header; anything shorter cannot be a binary (and would index past its end).
    if len(data) < 64 or data[:4] != b"\x7fELF" or data[4] != 2 or data[5] != 1:
        raise ValueError("not a 64-bit little-endian ELF file")
    (shoff,) = struct.unpack_from("<Q", data, 0x28)
    shentsize, shnum = struct.unpack_from("<HH", data, 0x3A)

    def string(offset):
        return data[offset : data.index(b"\0", offset)].decode()

    required, weak = [], []
    for index in range(shnum):
        _, kind, _, _, offset, _, link, count, _, _ = struct.unpack_from(
            "<IIQQQQIIQQ", data, shoff + index * shentsize
        )
        if kind != SHT_GNU_VERNEED:
            continue
        strings = struct.unpack_from("<IIQQQQIIQQ", data, shoff + link * shentsize)[4]
        entry = offset
        for _ in range(count):
            _, aux_count, _, aux_offset, next_entry = struct.unpack_from("<HHIII", data, entry)
            aux = entry + aux_offset
            for _ in range(aux_count):
                _, flags, _, name, next_aux = struct.unpack_from("<IHHII", data, aux)
                version = string(strings + name)
                if version.startswith("GLIBC_") and version[6:7].isdigit():
                    (weak if flags & VER_FLG_WEAK else required).append(version[6:])
                aux += next_aux
            entry += next_entry
    highest = lambda versions: max(versions, key=version_key) if versions else None
    return highest(required), highest(weak)


def main(argv):
    if len(argv) not in (2, 3):
        print(__doc__.split("\n")[2].strip(), file=sys.stderr)
        return 2
    path, maximum = argv[1], argv[2] if len(argv) == 3 else "2.35"
    try:
        with open(path, "rb") as file:
            required, weak = glibc_needs(file.read())
    except (OSError, ValueError, IndexError, struct.error) as error:  # a damaged file is an error, not a traceback
        print(f"error: {path}: {error}", file=sys.stderr)
        return 1
    note = f" (and weakly GLIBC_{weak}, which the loader does not insist on)" if weak and (
        required is None or version_key(weak) > version_key(required)
    ) else ""
    if required is None:
        print(f"{path} requires no glibc version{note}")
        return 0
    if version_key(required) > version_key(maximum):
        print(
            f"error: {path} needs GLIBC_{required}, newer than the GLIBC_{maximum} baseline.\n"
            "Build on the oldest supported distro (the ubuntu-22.04 runners), or raise the baseline on\n"
            "purpose in tools/check-glibc.py, release.yml, release-cache.yml and README.md.",
            file=sys.stderr,
        )
        return 1
    print(f"{path} needs GLIBC_{required} at most (baseline GLIBC_{maximum}): ok{note}")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
