"""Read Process Monitor logs and write Process Monitor configurations.

procmon_parser ships no type information, so the rest of the package uses
this module instead of importing procmon_parser.
"""

from dataclasses import dataclass
from typing import BinaryIO
lazy from collections.abc import Iterator

from procmon_parser import (
    Column,
    ProcmonLogsReader,
    Rule,
    RuleAction,
    RuleRelation,
    dump_configuration,
)


@dataclass(frozen=True, slots=True, kw_only=True)
class RegistryEvent:
    """One event from a Process Monitor log.

    Attributes:
        filetime: Event time in 100 ns ticks since 1601-01-01 UTC.
        process_name: Name of the process that caused the event.
        pid: ID of that process.
        operation: Process Monitor operation name, such as `RegSetValue`.
        path: Path of the registry value or other object.
        data: The value's bytes for a binary value, otherwise None.
        length: The value length Process Monitor reported, or None.
    """

    filetime: int
    process_name: str
    pid: int
    operation: str
    path: str
    data: bytes | None
    length: int | None


def read_events(log: BinaryIO) -> Iterator[RegistryEvent]:
    """Yield the events of a Process Monitor log opened in binary mode."""
    for event in ProcmonLogsReader(log):
        data = event.details.get("Data")
        length = event.details.get("Length")
        yield RegistryEvent(
            filetime=event.date_filetime,
            process_name=event.process.process_name,
            pid=event.process.pid,
            operation=event.operation.name,
            path=event.path,
            data=data if isinstance(data, bytes) else None,
            length=length if isinstance(length, int) else None,
        )


def write_cloudstore_filter(config: BinaryIO) -> None:
    """Write a configuration that keeps only CloudStore registry events.

    Process Monitor drops the other events instead of only hiding them, which
    keeps the capture file small.
    """
    rules = [
        Rule(Column.PATH, RuleRelation.CONTAINS, "CloudStore", RuleAction.INCLUDE),
        Rule(Column.PROCESS_NAME, RuleRelation.IS, "Procmon.exe", RuleAction.EXCLUDE),
        Rule(Column.PROCESS_NAME, RuleRelation.IS, "Procmon64.exe", RuleAction.EXCLUDE),
    ]
    dump_configuration({"FilterRules": rules, "DestructiveFilter": 1}, config)
