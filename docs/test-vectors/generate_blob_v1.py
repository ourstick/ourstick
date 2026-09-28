#!/usr/bin/env python3
"""Independent reference implementation of the OurStick blob format, version 1.

Generates blob-v1.json. It is written from docs/protocol.md alone, not from the
Rust code, so the Rust test that checks these vectors cross-checks both.

Requirements: pip install blake3 pycryptodome
Usage:        python3 generate_blob_v1.py > blob-v1.json
"""

import json

import blake3
from Crypto.Cipher import ChaCha20_Poly1305  # 24-byte nonce selects XChaCha20

BLOB_FORMAT_VERSION = 1
CIPHER_NONE = 0
CIPHER_XCHACHA20_POLY1305 = 1
CHUNK_SIZE = 64 * 1024
NONCE_PREFIX_LEN = 19
AEAD_KEY_CONTEXT = "OurStick 2026-09-28 blob XChaCha20-Poly1305 key"
NONCE_KEY_CONTEXT = "OurStick 2026-09-28 blob nonce prefix key"

# Blobs longer than this are given only by length and hash, not in full.
MAX_INLINE_BLOB = 1024


def derive_key(context: str, key: bytes) -> bytes:
    return blake3.blake3(key, derive_key_context=context).digest()


def split_chunks(plaintext: bytes) -> list[bytes]:
    if not plaintext:
        return [b""]
    return [plaintext[i : i + CHUNK_SIZE] for i in range(0, len(plaintext), CHUNK_SIZE)]


def pattern(length: int) -> bytes:
    return bytes(i % 251 for i in range(length))


def encrypt_xchacha(stick_key: bytes, plaintext: bytes) -> tuple[bytes, dict]:
    aead_key = derive_key(AEAD_KEY_CONTEXT, stick_key)
    nonce_key = derive_key(NONCE_KEY_CONTEXT, stick_key)
    plaintext_hash = blake3.blake3(plaintext).digest()
    prefix = blake3.blake3(plaintext_hash, key=nonce_key).digest()[:NONCE_PREFIX_LEN]
    header = bytes([BLOB_FORMAT_VERSION, CIPHER_XCHACHA20_POLY1305]) + prefix

    blob = bytearray(header)
    chunks = split_chunks(plaintext)
    nonces = []
    for index, chunk in enumerate(chunks):
        is_last = index == len(chunks) - 1
        nonce = prefix + index.to_bytes(4, "big") + bytes([1 if is_last else 0])
        nonces.append(nonce)
        aead = ChaCha20_Poly1305.new(key=aead_key, nonce=nonce)
        aead.update(header)  # AAD: the full header
        ciphertext, tag = aead.encrypt_and_digest(chunk)
        blob += ciphertext + tag

    intermediate = {
        "aead_key": aead_key.hex(),
        "nonce_key": nonce_key.hex(),
        "plaintext_hash": plaintext_hash.hex(),
        "nonce_prefix": prefix.hex(),
        "header": header.hex(),
        "first_chunk_nonce": nonces[0].hex(),
        "last_chunk_nonce": nonces[-1].hex(),
    }
    return bytes(blob), intermediate


def encrypt_none(plaintext: bytes) -> tuple[bytes, dict]:
    header = bytes([BLOB_FORMAT_VERSION, CIPHER_NONE])
    return header + plaintext, {"header": header.hex()}


def vector(name: str, cipher: str, stick_key: bytes | None, plaintext_spec: dict) -> dict:
    if "hex" in plaintext_spec:
        plaintext = bytes.fromhex(plaintext_spec["hex"])
    else:
        plaintext = pattern(plaintext_spec["pattern_length"])

    if cipher == "xchacha20-poly1305":
        blob, intermediate = encrypt_xchacha(stick_key, plaintext)
    else:
        blob, intermediate = encrypt_none(plaintext)

    result = {
        "name": name,
        "cipher": cipher,
        "stick_key": stick_key.hex() if stick_key else None,
        "plaintext": plaintext_spec,
        "plaintext_len": len(plaintext),
        **intermediate,
        "blob_len": len(blob),
        "blob_hash": blake3.blake3(blob).hexdigest(),
    }
    if len(blob) <= MAX_INLINE_BLOB:
        result["blob"] = blob.hex()
    return result


def main() -> None:
    key_counting = bytes(range(32))
    key_other = bytes.fromhex("f0e1d2c3b4a5968778695a4b3c2d1e0f00112233445566778899aabbccddeeff")
    short = "A USB stick you can send.".encode().hex()

    vectors = [
        vector("none-empty", "none", None, {"hex": ""}),
        vector("none-short", "none", None, {"hex": short}),
        vector("xchacha-empty", "xchacha20-poly1305", key_counting, {"hex": ""}),
        vector("xchacha-short", "xchacha20-poly1305", key_counting, {"hex": short}),
        vector("xchacha-short-other-key", "xchacha20-poly1305", key_other, {"hex": short}),
        vector("xchacha-one-full-chunk", "xchacha20-poly1305", key_counting,
               {"pattern_length": CHUNK_SIZE}),
        vector("xchacha-three-chunks", "xchacha20-poly1305", key_counting,
               {"pattern_length": 2 * CHUNK_SIZE + 1}),
    ]
    document = {
        "description": "OurStick blob format v1 test vectors. See docs/protocol.md.",
        "pattern": "pattern_length N means N bytes where byte i = i mod 251",
        "blob_format_version": BLOB_FORMAT_VERSION,
        "chunk_size": CHUNK_SIZE,
        "aead_key_context": AEAD_KEY_CONTEXT,
        "nonce_key_context": NONCE_KEY_CONTEXT,
        "vectors": vectors,
    }
    print(json.dumps(document, indent=2))


if __name__ == "__main__":
    main()
