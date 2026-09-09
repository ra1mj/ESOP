#!/usr/bin/env python3
"""Validate the versioned ESOP protobuf contract without a protobuf runtime."""

from __future__ import annotations

import pathlib
import re
import sys


ROOT = pathlib.Path(__file__).resolve().parents[1]
SCHEMA_ROOT = ROOT / "proto" / "esop" / "v1"
IDENTIFIER = re.compile(r"^[a-z][a-z0-9_]*$")
FIELD = re.compile(
    r"^(?:(?:repeated|optional)\s+)?(?:[A-Za-z_][A-Za-z0-9_<>., ]*)\s+"
    r"([a-z][a-z0-9_]*)\s*=\s*(\d+)\s*(?:\[[^]]*\])?;$"
)
ENUM_VALUE = re.compile(r"^([A-Z][A-Z0-9_]*)\s*=\s*(-?\d+)\s*;$")


def fail(message: str) -> None:
    raise ValueError(message)


def strip_comments(text: str) -> str:
    text = re.sub(r"/\*.*?\*/", "", text, flags=re.DOTALL)
    return re.sub(r"//[^\n]*", "", text)


def blocks(lines: list[str], keyword: str):
    index = 0
    while index < len(lines):
        match = re.match(rf"^\s*{keyword}\s+([A-Za-z_][A-Za-z0-9_]*)\s*\{{\s*$", lines[index])
        if not match:
            index += 1
            continue
        name = match.group(1)
        body: list[str] = []
        depth = 1
        index += 1
        while index < len(lines) and depth:
            line = lines[index]
            depth += line.count("{")
            depth -= line.count("}")
            if depth:
                body.append(line)
            index += 1
        if depth != 0:
            fail(f"unterminated {keyword} {name}")
        yield name, body


def validate_reserved(statement: str, reserved_numbers: set[int], reserved_names: set[str]) -> None:
    content = statement[len("reserved ") : -1].strip()
    for item in content.split(","):
        item = item.strip()
        if item.startswith('"') and item.endswith('"'):
            reserved_names.add(item[1:-1])
            continue
        if " to " in item:
            start, end = (int(part.strip()) for part in item.split(" to ", 1))
            reserved_numbers.update(range(start, end + 1))
            continue
        reserved_numbers.add(int(item))


def validate_message(name: str, body: list[str]) -> None:
    field_numbers: set[int] = set()
    field_names: set[str] = set()
    reserved_numbers: set[int] = set()
    reserved_names: set[str] = set()
    for raw_line in body:
        line = raw_line.strip()
        if not line:
            continue
        if line.startswith("reserved "):
            validate_reserved(line, reserved_numbers, reserved_names)
            continue
        if line in {"option optimize_for = SPEED;"}:
            continue
        match = FIELD.match(line)
        if not match:
            continue
        field_name, number_text = match.groups()
        number = int(number_text)
        if not IDENTIFIER.fullmatch(field_name):
            fail(f"{name}.{field_name}: field name must be snake_case")
        if number <= 0 or number >= 19000:
            fail(f"{name}.{field_name}: invalid field number {number}")
        if number in field_numbers:
            fail(f"{name}: duplicate field number {number}")
        if field_name in field_names:
            fail(f"{name}: duplicate field name {field_name}")
        field_numbers.add(number)
        field_names.add(field_name)
    overlap = field_numbers & reserved_numbers
    if overlap:
        fail(f"{name}: reserved field number reused: {sorted(overlap)}")
    name_overlap = field_names & reserved_names
    if name_overlap:
        fail(f"{name}: reserved field name reused: {sorted(name_overlap)}")


def validate_enum(name: str, body: list[str]) -> None:
    values: dict[str, int] = {}
    numbers: set[int] = set()
    for raw_line in body:
        line = raw_line.strip()
        if not line:
            continue
        match = ENUM_VALUE.match(line)
        if not match:
            continue
        value_name, number_text = match.groups()
        number = int(number_text)
        if value_name in values or number in numbers:
            fail(f"{name}: duplicate enum name or number")
        values[value_name] = number
        numbers.add(number)
    if 0 not in numbers:
        fail(f"{name}: missing zero unspecified value")
    zero_name = next(value_name for value_name, number in values.items() if number == 0)
    if not zero_name.endswith("_UNSPECIFIED"):
        fail(f"{name}: zero value must end in _UNSPECIFIED")


def validate_file(path: pathlib.Path) -> None:
    lines = strip_comments(path.read_text(encoding="utf-8")).splitlines()
    if not any(line.strip() == 'syntax = "proto3";' for line in lines):
        fail(f"{path}: syntax must be proto3")
    if not any(line.strip() == "package esop.v1;" for line in lines):
        fail(f"{path}: package must be esop.v1")
    for name, body in blocks(lines, "message"):
        validate_message(name, body)
    for name, body in blocks(lines, "enum"):
        validate_enum(name, body)


def main() -> int:
    try:
        files = sorted(SCHEMA_ROOT.glob("*.proto"))
        if not files:
            fail(f"no protobuf schemas found under {SCHEMA_ROOT}")
        for path in files:
            validate_file(path)
    except (OSError, ValueError) as error:
        print(f"protobuf schema invalid: {error}", file=sys.stderr)
        return 1
    print(f"protobuf schema valid: {len(files)} file(s), package esop.v1")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
