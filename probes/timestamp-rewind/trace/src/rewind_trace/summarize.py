"""Print the Night Light value reads and writes in a Process Monitor log as a summary.

Process Monitor records only the first 16 bytes of a value, so the value
length is the only sign of the preview flag: a settings value with preview on
is three bytes longer than the same value with preview off.
"""

import argparse
from datetime import UTC, datetime, timedelta
from pathlib import Path
from typing import Final
lazy from collections.abc import Iterable, Iterator

from rewind_trace.procmon import RegistryEvent, read_events
from rewind_trace.summary import Operation, Record, Store, SummaryRow, format_row

OPERATIONS: Final = frozendict(
    RegQueryValue=Operation.READ,
    RegSetValue=Operation.WRITE,
)
# The timestamp follows these bytes in each layout; see the probe's src/envelope.rs.
TIMESTAMP_MARKERS: Final = frozendict(
    {Store.CURRENT: bytes.fromhex("2a06"), Store.CLOUD: bytes.fromhex("0a0026")},
)
FILETIME_EPOCH: Final = datetime(1601, 1, 1, tzinfo=UTC)


def settings_timestamp(store: Store, data: bytes) -> int | None:
    """Return the settings timestamp in a settings value, or None if it has none."""
    marker = TIMESTAMP_MARKERS[store]
    index = data.find(marker, 4)
    if index < 0:
        return None
    value, shift = 0, 0
    for byte in data[index + len(marker) :]:
        value |= (byte & 0x7F) << shift
        if not byte & 0x80:
            return value
        shift += 7
    return None


def summarize(events: Iterable[RegistryEvent]) -> Iterator[SummaryRow]:
    """Yield a row for each Night Light value read or write in `events`.

    Skips reads without data, which only query the value's size.
    """
    first: int | None = None
    for event in events:
        operation = OPERATIONS.get(event.operation)
        record = _record(event.path)
        if operation is None or record is None:
            continue
        if operation is Operation.READ and event.data is None:
            continue
        store = Store.CLOUD if "\\Cloud\\" in event.path else Store.CURRENT
        if first is None:
            first = event.filetime
        stamp = None
        if record is Record.SETTINGS and event.data is not None:
            stamp = settings_timestamp(store, event.data)
        yield SummaryRow(
            ms=(event.filetime - first) / 10_000,
            process=event.process_name,
            pid=event.pid,
            operation=operation,
            store=store,
            record=record,
            stamp=stamp,
            length=event.length,
            utc=FILETIME_EPOCH + timedelta(microseconds=event.filetime // 10),
        )


def _record(path: str) -> Record | None:
    if path.endswith("windows.data.bluelightreduction.settings\\Data"):
        return Record.SETTINGS
    if path.endswith("windows.data.bluelightreduction.bluelightreductionstate\\Data"):
        return Record.STATE
    return None


def main() -> int:
    """Print one summary line per Night Light value read or write in a log."""
    parser = argparse.ArgumentParser(
        prog="summarize-trace",
        description=__doc__,
        formatter_class=argparse.RawDescriptionHelpFormatter,
    )
    parser.add_argument("log", type=Path, help="Process Monitor log (.pml)")
    arguments = parser.parse_args()
    with arguments.log.open("rb") as log:
        for row in summarize(read_events(log)):
            print(format_row(row))  # noqa: T201 -- the summary is this command's output
    return 0
