from typing import Any

import pytest

from spec_types import render_type


@pytest.mark.parametrize(
    ("node", "expected"),
    [
        ("address", "Address"),
        ("void", "()"),
        ("custom_scalar", "custom_scalar"),
        ({"option": {"value_type": "address"}}, "Option<Address>"),
        ({"vec": {"element_type": "u32"}}, "Vec<u32>"),
        ({"map": {"key_type": "symbol", "value_type": "i128"}}, "Map<Symbol, i128>"),
        ({"bytes_n": {"n": 32}}, "BytesN<32>"),
        ({"result": {"ok_type": "void", "error_type": "error"}}, "Result<(), error>"),
        ({"tuple": {"value_types": ["bool", "u64"]}}, "(bool, u64)"),
        ({"udt": {"name": "Attestation"}}, "Attestation"),
        (
            {"vec": {"element_type": {"option": {"value_type": {"bytes_n": {"n": 32}}}}}},
            "Vec<Option<BytesN<32>>>",
        ),
        ({"unknown": {"x": 1}}, '{"unknown": {"x": 1}}'),
    ],
)
def test_render_type(node: Any, expected: str) -> None:
    assert render_type(node) == expected
