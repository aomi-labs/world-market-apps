from __future__ import annotations

from dataclasses import dataclass, field
from decimal import Decimal

import jellyfish

from desk.cage.types import ResolvedInstrument


@dataclass
class InstrumentRow:
    symbol: str
    name: str
    product: str
    quote: str
    aliases: list[str]
    description: str
    last_price: Decimal
    adv: Decimal
    token_id: int | None = None
    confusable_group: str | None = None


# Always disambiguate by attribute when the query hits a group.
CONFUSABLE_GROUPS: dict[str, str] = {
    "ETH": "eth-family",
    "WETH": "eth-family",
    "ETHER": "eth-family",
    "ETHEREUM": "eth-family",
    "BTC": "btc-family",
    "WBTC": "btc-family",
    "BITCOIN": "btc-family",
    "SOL": "sol-family",
    "SOUL": "sol-family",
    "APP": "eth-family",
    "CISCO": "btc-family",
}


def default_universe() -> list[InstrumentRow]:
    return [
        InstrumentRow(
            symbol="WETH",
            name="Wrapped Ether",
            product="spot",
            quote="USDT",
            aliases=["eth", "ether", "ethereum", "weth", "wrapped ether", "w-e-t-h"],
            description="Wrapped Ether on World, around a few thousand USDT",
            last_price=Decimal("3800"),
            adv=Decimal("50000000"),
            token_id=None,
            confusable_group="eth-family",
        ),
        InstrumentRow(
            symbol="WETH",
            name="Ether perpetual",
            product="perp",
            quote="USDT",
            aliases=["eth perp", "ether perp", "ethereum perp", "weth perp"],
            description="USDT-margined ETH perpetual on World",
            last_price=Decimal("3800"),
            adv=Decimal("80000000"),
            confusable_group="eth-family",
        ),
        InstrumentRow(
            symbol="WBTC",
            name="Wrapped Bitcoin",
            product="spot",
            quote="USDT",
            aliases=["btc", "bitcoin", "wbtc", "wrapped bitcoin", "w-b-t-c"],
            description="Wrapped Bitcoin on World, around tens of thousands USDT",
            last_price=Decimal("95000"),
            adv=Decimal("40000000"),
            confusable_group="btc-family",
        ),
        InstrumentRow(
            symbol="WBTC",
            name="Bitcoin perpetual",
            product="perp",
            quote="USDT",
            aliases=["btc perp", "bitcoin perp", "wbtc perp"],
            description="USDT-margined BTC perpetual on World",
            last_price=Decimal("95000"),
            adv=Decimal("70000000"),
            confusable_group="btc-family",
        ),
        InstrumentRow(
            symbol="USDT",
            name="Tether",
            product="spot",
            quote="USDT",
            aliases=["usdt", "tether", "dollar", "dollars"],
            description="Quote asset on World",
            last_price=Decimal("1"),
            adv=Decimal("100000000"),
        ),
    ]


def _letters(query: str) -> str | None:
    stripped = query.strip().upper()
    if "-" in stripped and all(len(p) == 1 and p.isalpha() for p in stripped.split("-") if p):
        return stripped.replace("-", "")
    parts = stripped.split()
    if len(parts) >= 2 and all(len(p) == 1 and p.isalpha() for p in parts):
        return "".join(parts)
    return None


def _norm(s: str) -> str:
    return " ".join(s.lower().replace("-", " ").split())


@dataclass
class Candidate:
    instrument: ResolvedInstrument
    reason: str


@dataclass
class InstrumentResolver:
    rows: list[InstrumentRow] = field(default_factory=default_universe)

    def search(self, query: str, *, product: str | None = None) -> list[Candidate]:
        q = _norm(query)
        if not q:
            return []
        spelled = _letters(query)
        qkey = query.strip().upper().replace(" ", "").replace("-", "")
        if qkey in {"APP", "CISCO"}:
            group = CONFUSABLE_GROUPS[qkey]
            rows = [r for r in self.rows if r.confusable_group == group]
            return [self._cand(row, 0.7, "confusable") for row in rows]
        hits: list[Candidate] = []

        if spelled:
            for row in self._filter(product):
                if row.symbol == spelled:
                    hits.append(self._cand(row, 0.99, "spelled"))
            if hits:
                return self._maybe_confusable(query, hits)

        for row in self._filter(product):
            if _norm(row.symbol) == q:
                hits.append(self._cand(row, 0.98, "ticker"))
        if hits:
            return self._maybe_confusable(query, hits)

        for row in self._filter(product):
            aliases = [_norm(a) for a in row.aliases + [row.name]]
            if q in aliases:
                hits.append(self._cand(row, 0.95, "alias"))
        if hits:
            return self._maybe_confusable(query, hits)

        q_meta = jellyfish.metaphone(q)
        fuzzy: list[tuple[float, InstrumentRow, str]] = []
        for row in self._filter(product):
            names = [row.symbol, row.name, *row.aliases]
            best = 0.0
            for name in names:
                n = _norm(name)
                score = 0.0
                if q_meta and jellyfish.metaphone(n) == q_meta:
                    score = 0.72
                ratio = jellyfish.jaro_winkler_similarity(q, n)
                score = max(score, ratio * 0.9)
                best = max(best, score)
            if best >= 0.65:
                fuzzy.append((best, row, "phonetic"))
        fuzzy.sort(key=lambda t: t[0], reverse=True)
        hits = [self._cand(row, score, reason) for score, row, reason in fuzzy[:5]]
        return self._maybe_confusable(query, hits)

    def _filter(self, product: str | None) -> list[InstrumentRow]:
        if not product:
            return self.rows
        return [r for r in self.rows if r.product == product]

    def _cand(self, row: InstrumentRow, confidence: float, reason: str) -> Candidate:
        return Candidate(
            instrument=ResolvedInstrument(
                symbol=row.symbol,
                name=row.name,
                product=row.product,  # type: ignore[arg-type]
                quote=row.quote,
                token_id=row.token_id,
                confidence=min(confidence, 0.99),
                aliases=list(row.aliases),
                last_price=row.last_price,
                description=row.description,
            ),
            reason=reason,
        )

    def _maybe_confusable(self, query: str, hits: list[Candidate]) -> list[Candidate]:
        if not hits:
            return hits
        groups = {CONFUSABLE_GROUPS.get(_norm(query).upper().replace(" ", ""))}
        for hit in hits:
            groups.add(hit.instrument.aliases and None)
            key = hit.instrument.symbol
            groups.add(CONFUSABLE_GROUPS.get(key))
        qkey = query.strip().upper().replace(" ", "").replace("-", "")
        ambiguous_queries = {
            "ETH",
            "ETHER",
            "ETHEREUM",
            "BTC",
            "BITCOIN",
            "SOL",
            "SOUL",
            "APP",
            "CISCO",
        }
        group = CONFUSABLE_GROUPS.get(qkey) if qkey in ambiguous_queries else None
        if not group:
            return hits
        extra = [
            self._cand(row, 0.7, "confusable")
            for row in self.rows
            if row.confusable_group == group
            and all(
                not (c.instrument.symbol == row.symbol and c.instrument.product == row.product)
                for c in hits
            )
        ]
        if extra:
            # Force disambiguation: cap confidence so Cage cannot enter READBACK.
            for hit in hits:
                hit.instrument.confidence = min(hit.instrument.confidence, 0.85)
            return hits + extra
        return hits

    def keyterms(self, extra: list[str] | None = None) -> list[str]:
        terms: list[str] = []
        for row in self.rows:
            terms.append(row.symbol)
            terms.append(row.name)
            terms.extend(row.aliases)
        terms.extend(extra or [])
        # cap 100
        seen: list[str] = []
        for t in terms:
            if t not in seen:
                seen.append(t)
            if len(seen) >= 100:
                break
        return seen
