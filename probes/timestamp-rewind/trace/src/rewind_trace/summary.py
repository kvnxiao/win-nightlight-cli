"""Write and parse trace summaries: one tab-separated line per Night Light value access.

Each line has, in order: milliseconds since the first event, process name,
process ID, operation, value (store and record, such as `Current/settings`),
settings timestamp, value length in bytes, and UTC time. A summary can lack
the last column or the last two; `Summary` records which columns it has.
"""

import csv
from dataclasses import dataclass
from datetime import datetime
from enum import StrEnum
from typing import Final
lazy from collections.abc import Iterable

PROBE_PROCESS_PREFIX: Final = "win-nightlight"
REQUIRED_COLUMNS: Final = 6
LENGTH_COLUMNS: Final = 7
UTC_COLUMNS: Final = 8


class Operation(StrEnum):
    """Registry access in a summary line."""

    READ = "read"
    WRITE = "write"


class Store(StrEnum):
    """CloudStore subtree that stores a value."""

    CURRENT = "Current"
    CLOUD = "Cloud"


class Record(StrEnum):
    """Night Light record that a value stores."""

    SETTINGS = "settings"
    STATE = "state"


@dataclass(frozen=True, slots=True, kw_only=True)
class SummaryRow:
    """One Night Light value read or write.

    Attributes:
        ms: Milliseconds since the first event of the summary.
        process: Name of the process that read or wrote the value.
        pid: ID of that process.
        operation: Whether the process read or wrote the value.
        store: CloudStore subtree of the value.
        record: Night Light record of the value.
        stamp: Settings timestamp in the value, or None for a state value or a
            value without one.
        length: Value length in bytes, or None when the summary has no length.
        utc: Event time, or None when the summary has no UTC time.
    """

    ms: float
    process: str
    pid: int
    operation: Operation
    store: Store
    record: Record
    stamp: int | None
    length: int | None
    utc: datetime | None

    @property
    def is_settings_write(self) -> bool:
        """Return whether the row writes the `Current` settings value."""
        return (
            self.operation is Operation.WRITE
            and self.store is Store.CURRENT
            and self.record is Record.SETTINGS
        )


@dataclass(frozen=True, slots=True, kw_only=True)
class Summary:
    """Rows of a summary and the optional columns it has."""

    rows: tuple[SummaryRow, ...]
    has_lengths: bool
    has_utc: bool


def format_row(row: SummaryRow) -> str:
    """Return the summary line for `row`, without a line terminator."""
    return "\t".join(
        (
            f"{row.ms:.1f}",
            row.process,
            str(row.pid),
            row.operation,
            f"{row.store}/{row.record}",
            _optional(row.stamp),
            _optional(row.length),
            "" if row.utc is None else row.utc.isoformat(),
        ),
    )


def parse_summary(lines: Iterable[str]) -> Summary:
    """Parse summary lines.

    Raises:
        ValueError: A line has fewer than six columns, or a column does not
            have the format `format_row` writes.
    """
    fields = list(csv.reader(lines, delimiter="\t"))
    columns = min((len(row) for row in fields), default=UTC_COLUMNS)
    return Summary(
        rows=tuple(_parse_row(row) for row in fields),
        has_lengths=columns >= LENGTH_COLUMNS,
        has_utc=columns >= UTC_COLUMNS,
    )


def _parse_row(fields: list[str]) -> SummaryRow:
    if len(fields) < REQUIRED_COLUMNS:
        message = f"expected at least {REQUIRED_COLUMNS} columns, got {len(fields)}"
        raise ValueError(message)
    store, record = fields[4].split("/")
    length = fields[6] if len(fields) >= LENGTH_COLUMNS else ""
    utc = fields[7] if len(fields) >= UTC_COLUMNS else ""
    return SummaryRow(
        ms=float(fields[0]),
        process=fields[1],
        pid=int(fields[2]),
        operation=Operation(fields[3]),
        store=Store(store),
        record=Record(record),
        stamp=int(fields[5]) if fields[5] else None,
        length=int(length) if length else None,
        utc=datetime.fromisoformat(utc) if utc else None,
    )


def _optional(value: int | None) -> str:
    return "" if value is None else str(value)
