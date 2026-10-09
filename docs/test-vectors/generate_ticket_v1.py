#!/usr/bin/env python3
"""Independent reference implementation of the OurStick ticket and signed
manifest record formats, version 1.

Generates ticket-v1.json. It is written from docs/protocol.md alone, not from the
Rust code, so the Rust test that checks these vectors cross-checks both.

Requirements: pip install blake3 pycryptodome
Usage:        python3 generate_ticket_v1.py > ticket-v1.json
"""

import base64
import ipaddress
import json

import blake3
from Crypto.PublicKey import ECC
from Crypto.Signature import eddsa

TICKET_FORMAT_VERSION = 1
RECORD_FORMAT_VERSION = 1
LINK_PREFIX = "https://ourstick.com/s/#"
RECORD_CONTEXT = b"OurStick manifest record v1\x00"


def varint(value: int) -> bytes:
    """Postcard varint (unsigned LEB128)."""
    out = bytearray()
    while True:
        byte = value & 0x7F
        value >>= 7
        if value:
            out.append(byte | 0x80)
        else:
            out.append(byte)
            return bytes(out)


def postcard_hint(hint: dict) -> bytes:
    if hint["kind"] == "ipv4":
        return varint(0) + ipaddress.IPv4Address(hint["ip"]).packed + varint(hint["port"])
    if hint["kind"] == "ipv6":
        return varint(1) + ipaddress.IPv6Address(hint["ip"]).packed + varint(hint["port"])
    url = hint["url"].encode("ascii")
    return varint(2) + varint(len(url)) + url


def check(*parts: bytes) -> bytes:
    return blake3.blake3(b"".join(parts)).digest()[:4]


def base32(data: bytes) -> str:
    return base64.b32encode(data).decode("ascii").rstrip("=").lower()


def signing_key(seed: bytes):
    return ECC.construct(curve="Ed25519", seed=seed)


def stick_id(seed: bytes) -> bytes:
    return signing_key(seed).public_key().export_key(format="raw")


def ticket_vector(name: str, seed: bytes, node_id: bytes, manifest_seq: int,
                  manifest_hash: bytes, hints: list[dict], stick_key: bytes) -> dict:
    body = (stick_id(seed) + node_id + varint(manifest_seq) + manifest_hash
            + varint(len(hints)))
    body += b"".join(postcard_hint(hint) for hint in hints)
    public = bytes([TICKET_FORMAT_VERSION]) + body
    public += check(public)
    secret = stick_key + check(public, stick_key)
    return {
        "name": name,
        "signing_seed": seed.hex(),
        "stick_id": stick_id(seed).hex(),
        "node_id": node_id.hex(),
        "manifest_seq": manifest_seq,
        "manifest_hash": manifest_hash.hex(),
        "address_hints": hints,
        "stick_key": stick_key.hex(),
        "public_part": public.hex(),
        "secret_part": secret.hex(),
        "link": f"{LINK_PREFIX}{base32(public)}.{base32(secret)}",
    }


def against_ticket(seq: int, manifest_hash: bytes, ticket_seq: int, ticket_hash: bytes) -> str:
    """Outcome of checking a record against a ticket's manifest hint (no seq seen before)."""
    if seq < ticket_seq:
        return "older_than_ticket"
    if seq == ticket_seq and manifest_hash != ticket_hash:
        return "conflicts_with_ticket"
    return "accept"


def record_vector(name: str, seed: bytes, seq: int, manifest_hash: bytes,
                  ticket_seq: int, ticket_hash: bytes) -> dict:
    signed_part = (bytes([RECORD_FORMAT_VERSION]) + stick_id(seed)
                   + seq.to_bytes(8, "big") + manifest_hash)
    message = RECORD_CONTEXT + signed_part
    signature = eddsa.new(signing_key(seed), "rfc8032").sign(message)
    return {
        "name": name,
        "signing_seed": seed.hex(),
        "stick_id": stick_id(seed).hex(),
        "seq": seq,
        "manifest_hash": manifest_hash.hex(),
        "message": message.hex(),
        "signature": signature.hex(),
        "record": (signed_part + signature).hex(),
        "against_ticket": against_ticket(seq, manifest_hash, ticket_seq, ticket_hash),
    }


def main() -> None:
    seed = bytes(range(32))
    node_id = blake3.blake3(b"node").digest()
    manifest_seq = 300  # two-byte varint
    manifest_hash = blake3.blake3(b"manifest").digest()
    other_hash = blake3.blake3(b"other manifest").digest()
    stick_key = blake3.blake3(b"stick key").digest()
    hints = [
        {"kind": "ipv4", "ip": "192.168.1.20", "port": 4433},
        {"kind": "ipv6", "ip": "2001:db8::1", "port": 4433},
        {"kind": "relay", "url": "https://relay.example.org./"},
    ]

    document = {
        "description": "OurStick ticket and signed manifest record v1 test vectors. "
                       "See docs/protocol.md.",
        "ticket_format_version": TICKET_FORMAT_VERSION,
        "record_format_version": RECORD_FORMAT_VERSION,
        "tickets": [
            ticket_vector("with-hints", seed, node_id, manifest_seq, manifest_hash, hints,
                          stick_key),
            ticket_vector("no-hints", seed, node_id, manifest_seq, manifest_hash, [],
                          stick_key),
        ],
        "records": [
            # Each record is also checked against the tickets' hint (seq 300, manifest_hash).
            record_vector("seq-1", seed, 1, manifest_hash, manifest_seq, manifest_hash),
            record_vector("seq-300", seed, 300, manifest_hash, manifest_seq, manifest_hash),
            record_vector("seq-300-other-hash", seed, 300, other_hash, manifest_seq,
                          manifest_hash),
            record_vector("seq-large", seed, 0x0102030405060708, other_hash, manifest_seq,
                          manifest_hash),
        ],
    }
    print(json.dumps(document, indent=2))


if __name__ == "__main__":
    main()
