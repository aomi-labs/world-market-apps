from decimal import Decimal

import pytest

from desk.speech import (
    digit_group,
    speak_decimal,
    speak_grouped_int,
    speak_int,
    speak_price,
    speak_quantity,
    speak_text,
    spell_ticker,
)

CASES = [
    ("$185.50", "expert", "one eighty-five fifty"),
    ("$185.50", "novice", "one hundred eighty-five dollars and fifty cents"),
    ("$1.05", "expert", "one oh five"),
    ("$1.05", "novice", "one dollars and five cents"),
    ("$0.50", "expert", "zero fifty"),
    ("$200", "expert", "two hundred"),
    ("$200", "novice", "two hundred dollars"),
    ("$3800", "expert", "thirty-eight hundred"),
    ("$1000", "expert", "one thousand"),
    ("$1000000", "expert", "one million"),
    ("185 USDT", "expert", "one eighty-five"),
    ("12 dollars", "novice", "twelve dollars"),
    ("[spell:WETH]", "expert", "W E T H"),
    ("[digits:1500]", "expert", "one-five-zero-zero"),
    ("hello", "expert", "hello"),
    ("$2.00", "expert", "two"),
    ("$99.99", "expert", "ninety-nine ninety-nine"),
    ("$21", "expert", "twenty-one"),
    ("$110", "expert", "one ten"),
    ("$101", "expert", "one oh one"),
    ("$3.09", "expert", "three oh nine"),
    ("$40", "expert", "forty"),
    ("$80.00", "expert", "eighty"),
    ("$250", "expert", "two fifty"),
    ("$999", "expert", "nine ninety-nine"),
    ("$1001", "expert", "one thousand one"),
    ("$12,000", "expert", "twelve thousand"),
    ("$0.01", "expert", "zero oh one"),
    ("$7.10", "expert", "seven ten"),
    ("$60.60", "expert", "sixty sixty"),
    ("1 dollars", "expert", "one"),
    ("250 USD", "expert", "two fifty"),
    ("[spell:WBTC]", "expert", "W B T C"),
    ("[spell:AAPL]", "expert", "A A P L"),
    ("[digits:1000]", "expert", "one-zero-zero-zero"),
    ("[digits:42]", "expert", "four-two"),
    ("$30", "novice", "thirty dollars"),
    ("$30.50", "novice", "thirty dollars and fifty cents"),
    ("$1,850", "expert", "one thousand eight fifty"),
    ("$2,000,000", "expert", "two million"),
    ("$15.15", "expert", "fifteen fifteen"),
    ("$16", "expert", "sixteen"),
    ("$17.00", "novice", "seventeen dollars"),
    ("$18.18", "expert", "eighteen eighteen"),
    ("$19", "expert", "nineteen"),
    ("$22.22", "expert", "twenty-two twenty-two"),
    ("$33", "expert", "thirty-three"),
    ("$44.04", "expert", "forty-four oh four"),
    ("$55", "expert", "fifty-five"),
    ("$70.07", "expert", "seventy oh seven"),
]


@pytest.mark.parametrize("src,verbosity,expect", CASES)
def test_speak_text_table(src, verbosity, expect):
    assert speak_text(src, verbosity=verbosity) == expect


def test_speak_int_branches():
    assert speak_int(0) == "zero"
    assert speak_int(7) == "seven"
    assert speak_int(15) == "fifteen"
    assert speak_int(20) == "twenty"
    assert speak_int(21) == "twenty-one"
    assert speak_int(100) == "one hundred"
    assert speak_int(101) == "one oh one"
    assert speak_int(115) == "one hundred fifteen"
    assert speak_int(-3) == "minus three"
    assert speak_int(2000) == "two thousand"
    assert speak_int(2001) == "two thousand one"
    assert speak_int(1_000_000) == "one million"
    assert speak_int(1_000_007) == "one million seven"


def test_grouped_and_price():
    assert speak_grouped_int(-12) == "minus twelve"
    assert speak_grouped_int(50) == "fifty"
    assert speak_grouped_int(185) == "one eighty-five"
    assert speak_grouped_int(3800) == "thirty-eight hundred"
    assert speak_grouped_int(3801) == "three thousand eight oh one"
    assert speak_grouped_int(1001) == "one thousand one"
    assert speak_grouped_int(12_500) == "twelve thousand five hundred"
    assert speak_price(Decimal("-1.50"), verbosity="expert").startswith("minus")
    assert "dollars" in speak_price(Decimal("2"), verbosity="novice")
    assert speak_decimal(Decimal("0.25")) == "point two five"
    assert speak_decimal(Decimal("3")) == "three"
    assert speak_decimal(Decimal("3.5"), verbosity="novice").startswith("three")
    assert speak_decimal(Decimal("-0.2")).startswith("minus")
    assert spell_ticker("weth") == "W E T H"
    assert digit_group(1500) == "one five zero zero"


def test_quantity_low_confidence_digit_groups():
    spoken = speak_quantity(
        Decimal("1500"),
        unit="wrapped ether",
        verbosity="expert",
        confidence=0.4,
        readback=True,
    )
    assert "one-five-zero-zero" in spoken or "one five zero zero" in spoken.replace("-", " ")
    assert "wrapped ether" in spoken
    half = speak_quantity(Decimal("0.5"), unit="ether", verbosity="expert", confidence=1, readback=True)
    assert "point five" in half
    novice_half = speak_quantity(Decimal("0.5"), unit="ether", verbosity="novice", confidence=1, readback=True)
    assert "one half" in novice_half


def test_readback_rewrites_large_qty():
    text = speak_text(
        "Sell 1500 wrapped ether",
        state="READBACK",
        slot_confidence={"quantity": 0.4},
    )
    assert "one thousand five hundred" in text or "that's" in text


def test_spell_tickers_flag():
    out = speak_text("Buy WETH now", spell_tickers=frozenset({"WETH"}))
    assert "W E T H" in out


def test_plain_money_and_cents_branch():
    assert "fifty" in speak_text("$10.50")
    assert speak_text("$3.09").endswith("nine") or "oh nine" in speak_text("$3.09")
    assert speak_grouped_int(1_250_000) == speak_int(1_250_000)
    assert "USDT" in speak_text("Buy USDT now", spell_tickers=frozenset({"WETH"}))
    assert speak_decimal(Decimal("-2.5")).startswith("minus")
    assert "million" in speak_int(2_000_007)
    assert speak_quantity(Decimal("3"), unit="ether", verbosity="expert", confidence=1, readback=False)
    novice = speak_price(Decimal("-2.50"), verbosity="novice")
    assert novice.startswith("minus")
    assert speak_grouped_int(200) == "two hundred"
    assert speak_grouped_int(12_050).startswith("twelve thousand")
