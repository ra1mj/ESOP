#!/usr/bin/env python3
"""Regenerate all software master products and compare their six frozen artifacts."""
import argparse
import os
from pathlib import Path
import subprocess

ROOT = Path(__file__).resolve().parents[1]
ARTIFACTS = ("device_inventory.json", "esop_product_config.h", "esop_product_config.rs",
             "procbuf_layout.json", "product_config.json", "robot_build_input.json")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--cargo", default=os.environ.get("CARGO", "cargo"))
    args = parser.parse_args()
    products = [(f"dual-{mode}", ROOT / "config/examples/sim-r0-r2" /
                 ("product.json" if mode == "csp" else f"product-{mode}.json"),
                 ROOT / f"config/examples/sim-r0-r2/expected/{mode}")
                for mode in ("csp", "csv", "cst")]
    products += [(grade, ROOT / f"config/examples/sim-r0-r2-load/{grade}/product.json",
                  ROOT / f"config/examples/sim-r0-r2-load/{grade}/expected")
                 for grade in ("q1", "q2", "q4")]
    for name, product, expected in products:
        output = ROOT / "build/generated/master" / name
        subprocess.run([args.cargo, "run", "--quiet", "-p", "esop-cfggen", "--",
                        "--input", str(product), "--output", str(output)], cwd=ROOT, check=True)
        for artifact in ARTIFACTS:
            if (output / artifact).read_bytes() != (expected / artifact).read_bytes():
                raise SystemExit(f"stale master product: {expected / artifact}")
        subprocess.run(["gcc", "-std=c11", "-Wall", "-Wextra", "-Werror", "-x", "c",
                        "-fsyntax-only", str(output / "esop_product_config.h")], check=True)
    print("all six software master product sets match their frozen artifacts")


if __name__ == "__main__":
    main()
