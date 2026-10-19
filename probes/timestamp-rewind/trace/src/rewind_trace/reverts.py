"""Print which processes reverted the probe's settings writes below Cloud, and when.

A probe write is below Cloud when its timestamp is below the Cloud timestamp
most recently read or written in the trace. A revert is a settings write by
another process, stamped with that Cloud timestamp, before the probe's next
write below Cloud. With a results file, only probe writes made during that
run count. The output gives each process's revert count and delays in
milliseconds after the probe write.
"""

import statistics
from dataclasses import dataclass, field
from typing import Final
lazy from collections.abc import Iterable
lazy from datetime import datetime

from rewind_trace.summary import (
    PROBE_PROCESS_PREFIX,
    Record,
    Store,
    SummaryRow,
)
from rewind_trace.trace_input import in_window, load_trace_input

# A revert later than this counts as late; the usual revert lands within 4 ms.
LATE_MS: Final = 10.0


@dataclass(frozen=True, slots=True, kw_only=True)
class RevertReport:
    """Probe writes below Cloud and the reverts that followed them.

    Attributes:
        below_cloud: Number of probe writes below Cloud.
        delays: Each reverting process's delays after the probe write, in
            milliseconds, in ascending order.
    """

    below_cloud: int
    delays: frozendict[str, tuple[float, ...]]


@dataclass(slots=True, kw_only=True)
class _PendingRevert:
    written_ms: float
    cloud: int
    reverted_by: set[str] = field(default_factory=set)


def find_reverts(
    rows: Iterable[SummaryRow],
    window: tuple[datetime, datetime] | None,
) -> RevertReport:
    """Find the reverts of the probe's writes below Cloud.

    Args:
        rows: Summary rows of a trace, in trace order.
        window: When given, count only probe writes made between these times.
            Rows without a UTC time are then outside it.
    """
    cloud: int | None = None
    pending: _PendingRevert | None = None
    delays: dict[str, list[float]] = {}
    below_cloud = 0
    for row in rows:
        if (
            row.store is Store.CLOUD
            and row.record is Record.SETTINGS
            and row.stamp is not None
        ):
            cloud = row.stamp
        if not row.is_settings_write or row.stamp is None or cloud is None:
            continue
        if row.process.startswith(PROBE_PROCESS_PREFIX):
            if row.stamp < cloud:
                pending = None
                if in_window(row, window):
                    below_cloud += 1
                    pending = _PendingRevert(written_ms=row.ms, cloud=cloud)
            continue
        if (
            pending is not None
            and row.stamp == pending.cloud
            and row.process not in pending.reverted_by
        ):
            pending.reverted_by.add(row.process)
            delays.setdefault(row.process, []).append(row.ms - pending.written_ms)
    return RevertReport(
        below_cloud=below_cloud,
        delays=frozendict(
            {process: tuple(sorted(values)) for process, values in delays.items()},
        ),
    )


def format_report(report: RevertReport) -> list[str]:
    """Return the report lines, one per reverting process after the total."""
    lines = [f"probe writes below Cloud: {report.below_cloud}"]
    for process, values in sorted(report.delays.items()):
        late = [round(value, 1) for value in values if value > LATE_MS]
        lines.append(
            f"{process}: reverted {len(values)}, "
            f"median {statistics.median(values):.1f} ms, max {values[-1]:.1f} ms, "
            f"later than {LATE_MS:.0f} ms: {len(late)} {late}",
        )
    return lines


def main() -> int:
    """Print each process's revert count and delays for probe writes below Cloud."""
    trace = load_trace_input("reverts", __doc__)
    for line in format_report(find_reverts(trace.summary.rows, trace.window)):
        print(line)  # noqa: T201 -- the report is this command's output
    return 0
