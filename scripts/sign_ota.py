#!/usr/bin/env python3
"""
sign_ota.py — 为 OTA 包的 meta.json 生成 Ed25519 签名。

固件侧（ota.rs）会校验 meta.json 中的 `signature` 字段：
    signature = base64( Ed25519_sign( "version|arch|binary_md5" ) )
验签公钥编译进固件（OTA_PUBKEY），私钥仅由本脚本 / CI 持有。

用法：
    python3 sign_ota.py \
        --meta  path/to/meta.json \
        --binary path/to/udx710 \
        --key   scripts/ota_sign_key.bin \
        [--out  path/to/signed_meta.json]   # 默认原地写回 meta.json

前置：pip install cryptography
"""
import argparse
import base64
import hashlib
import json
import sys

from cryptography.hazmat.primitives.asymmetric.ed25519 import Ed25519PrivateKey
from cryptography.hazmat.primitives import serialization


def md5_hex(path: str) -> str:
    h = hashlib.md5()
    with open(path, "rb") as f:
        for chunk in iter(lambda: f.read(1 << 20), b""):
            h.update(chunk)
    return h.hexdigest()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--meta", required=True, help="meta.json 路径")
    ap.add_argument("--binary", required=True, help="udx710 二进制路径（用于计算 MD5）")
    ap.add_argument("--key", required=True, help="Ed25519 私钥原始 32 字节文件")
    ap.add_argument("--out", default=None, help="输出 meta.json（默认原地覆盖）")
    args = ap.parse_args()

    with open(args.key, "rb") as f:
        priv_raw = f.read()
    if len(priv_raw) != 32:
        print(f"ERROR: 私钥长度应为 32 字节，实际 {len(priv_raw)}", file=sys.stderr)
        return 1
    sk = Ed25519PrivateKey.from_private_bytes(priv_raw)

    with open(args.meta, "r", encoding="utf-8") as f:
        meta = json.load(f)

    binary_md5 = md5_hex(args.binary)
    meta["binary_md5"] = binary_md5

    version = meta.get("version", "")
    arch = meta.get("arch", "")
    msg = f"{version}|{arch}|{binary_md5}".encode("utf-8")
    sig = sk.sign(msg)
    meta["signature"] = base64.b64encode(sig).decode("ascii")

    out_path = args.out or args.meta
    with open(out_path, "w", encoding="utf-8") as f:
        json.dump(meta, f, ensure_ascii=False, indent=2)
        f.write("\n")

    print(f"OK: signed meta.json -> {out_path}")
    print(f"     version={version} arch={arch} binary_md5={binary_md5}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
