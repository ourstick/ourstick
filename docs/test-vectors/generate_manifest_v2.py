#!/usr/bin/env python3
"""Generates manifest-v2.json: manifest wire format 2 (docs/protocol.md), written
independently of the Rust code from the postcard rules it relies on:

- bool: one byte, 0 or 1;
- u64 and lengths: unsigned LEB128 ("varint");
- string: varint byte length, then UTF-8;
- [u8; 32]: 32 raw bytes;
- a struct: its fields in order; a Vec: varint count, then the items.

Run: python3 generate_manifest_v2.py > manifest-v2.json
"""

import json


def varint(value):
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def string(text):
    data = text.encode("utf-8")
    return varint(len(data)) + data


def entry(path, size, mtime, hash_byte):
    return string(path) + varint(size) + varint(mtime) + bytes([hash_byte]) * 32


def manifest_v2(partial, name, entries):
    body = bytes([1 if partial else 0]) + string(name) + varint(len(entries))
    for item in entries:
        body += entry(*item)
    return bytes([2]) + body


def manifest_v1(entries):
    body = varint(len(entries))
    for item in entries:
        body += entry(*item)
    return bytes([1]) + body


VALID = [
    ("empty", False, "", []),
    (
        "partial-named",
        True,
        "Για τη Μαρία",
        [
            ("a.txt", 5, 1_700_000_000, 0x11),
            ("photos/2024/cat.jpg", 300, 1_234_567_890, 0x22),
            ("videos/big.mkv", 3_756_724_161, 0, 0x33),
        ],
    ),
    ("complete-sixty", False, "x" * 60, [("z", 0, 1, 0x44)]),
]

INVALID = [
    ("name-too-long", manifest_v2(False, "x" * 61, [])),
    ("name-control", manifest_v2(False, "tab\there", [])),
    ("name-bidi-override", manifest_v2(False, "‮evil", [])),
    ("name-zero-width", manifest_v2(False, "a​b", [])),
    ("name-line-separator", manifest_v2(False, "a b", [])),
    ("version-3", bytes([3]) + manifest_v2(False, "", [])[1:]),
]


def main():
    document = {
        "description": "OurStick manifest wire format 2 test vectors. See docs/protocol.md.",
        "manifest_format_version": 2,
        "valid": [
            {
                "name": case,
                "partial": partial,
                "stick_name": name,
                "entries": [
                    {"path": p, "size": s, "mtime": m, "hash": f"{h:02x}" * 32}
                    for (p, s, m, h) in entries
                ],
                "bytes": manifest_v2(partial, name, entries).hex(),
            }
            for (case, partial, name, entries) in VALID
        ],
        "version_1": {
            "entries": [{"path": "a.txt", "size": 5, "mtime": 7, "hash": "55" * 32}],
            "bytes": manifest_v1([("a.txt", 5, 7, 0x55)]).hex(),
        },
        "invalid": [{"name": case, "bytes": data.hex()} for (case, data) in INVALID],
    }
    print(json.dumps(document, indent=2, ensure_ascii=False))


if __name__ == "__main__":
    main()
