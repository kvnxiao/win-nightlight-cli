"""Print whether explorer.exe read each of the probe's settings writes, and how soon.

explorer.exe read a probe write when it read the `Current` settings value with
that write's timestamp. A rewind can repeat a timestamp, so a read matches the
most recent unread probe write with its timestamp. A later write by another
process with the same timestamp ends that match, because a read of the
timestamp can then be of either write. With a results file, only probe writes
made during that run count. The output gives the number of writes
read; the median, 90th percentile, and largest delay from a write to its
first read, in milliseconds; and the longest run of consecutive writes that
explorer.exe did not read.
"""

import statistics
from dataclasses import dataclass
from typing import Final
lazy from collections.abc import Iterable
lazy from datetime import datetime

from rewind_trace.summary import (
    PROBE_PROCESS_PREFIX,
    Operation,
    Record,
    Store,
    SummaryRow,
)
from rewind_trace.trace_input import in_window, load_trace_input

READER: Final = "explorer.exe"


@dataclass(frozen=True, slots=True, kw_only=True)
class ReadReport:
    """The probe's settings writes and how soon explorer.exe read each.

    Attributes:
        lags: For each probe write in trace order, the milliseconds from the
            write to explorer.exe's first read of it, or None if explorer.exe
            did not read it.
    """

    lags: tuple[float | None, ...]


def find_reads(
    rows: Iterable[SummaryRow],
    window: tuple[datetime, datetime] | None,
) -> ReadReport:
    """Match the probe's settings writes with explorer.exe's reads of them.

    Args:
        rows: Summary rows of a trace, in trace order.
        window: When given, count only probe writes made between these times.
            Rows without a UTC time are then outside it.
    """
    lags: list[float | None] = []
    unread: dict[int, tuple[int, float]] = {}
    for row in rows:
        if (
            row.store is not Store.CURRENT
            or row.record is not Record.SETTINGS
            or row.stamp is None
        ):
            continue
        if row.is_settings_write and row.process.startswith(PROBE_PROCESS_PREFIX):
            if in_window(row, window):
                unread[row.stamp] = (len(lags), row.ms)
                lags.append(None)
        elif row.is_settings_write:
            unread.pop(row.stamp, None)
        elif row.operation is Operation.READ and row.process.casefold() == READER:
            match = unread.pop(row.stamp, None)
            if match is not None:
                index, written_ms = match
                lags[index] = row.ms - written_ms
    return ReadReport(lags=tuple(lags))


def format_report(report: ReadReport) -> list[str]:
    """Return the report lines."""
    read = [lag for lag in report.lags if lag is not None]
    lines = [
        f"probe settings writes: {len(report.lags)}",
        f"read by {READER}: {len(read)}",
    ]
    if read:
        deciles = statistics.quantiles(read, n=10, method="inclusive")
        lines.append(
            f"delay to first read: median {statistics.median(read):.1f} ms, "
            f"90th percentile {deciles[-1]:.1f} ms, max {max(read):.1f} ms",
        )
    longest = current = 0
    for lag in report.lags:
        current = current + 1 if lag is None else 0
        longest = max(longest, current)
    lines.append(f"longest run of unread writes: {longest}")
    return lines


def main() -> int:
    """Print how many of the probe's settings writes explorer.exe read, and how soon."""
    trace = load_trace_input("write-reads", __doc__)
    for line in format_report(find_reads(trace.summary.rows, trace.window)):
        print(line)  # noqa: T201 -- the report is this command's output
    return 0
