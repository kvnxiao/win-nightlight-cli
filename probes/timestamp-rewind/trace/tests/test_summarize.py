from datetime import UTC, datetime

import pytest

from rewind_trace.procmon import RegistryEvent
from rewind_trace.summarize import summarize, value_timestamp
from rewind_trace.summary import Operation, Record, Store, SummaryRow

CURRENT_SETTINGS = (
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\CloudStore\\Store\\"
    "DefaultAccount\\Current\\default$windows.data.bluelightreduction.settings\\"
    "windows.data.bluelightreduction.settings\\Data"
)
CLOUD_SETTINGS = CURRENT_SETTINGS.replace("\\Current\\", "\\Cloud\\")
CURRENT_STATE = (
    "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\CloudStore\\Store\\"
    "DefaultAccount\\Current\\"
    "default$windows.data.bluelightreduction.bluelightreductionstate\\"
    "windows.data.bluelightreduction.bluelightreductionstate\\Data"
)
# Cloud record example from docs/nightlight-registry-format.md.
CLOUD_VALUE = bytes.fromhex("434201000A0026F590D5D60600")
# Envelope header, metadata struct, payload container, and timestamp field.
CURRENT_VALUE = bytes.fromhex("434201000A0201002A06F590D5D6062A")
# 2026-10-19T02:10:03 UTC in 100 ns ticks since 1601-01-01.
START_FILETIME = 134_368_494_030_000_000


def event(
    *,
    filetime: int = START_FILETIME,
    operation: str = "RegSetValue",
    path: str = CURRENT_SETTINGS,
    data: bytes | None = CURRENT_VALUE,
) -> RegistryEvent:
    return RegistryEvent(
        filetime=filetime,
        process_name="explorer.exe",
        pid=4321,
        operation=operation,
        path=path,
        data=data,
        length=None if data is None else len(data),
    )


@pytest.mark.parametrize(
    ("store", "data", "expected"),
    [
        pytest.param(Store.CLOUD, CLOUD_VALUE, 1792362613, id="cloud"),
        pytest.param(Store.CURRENT, CURRENT_VALUE, 1792362613, id="current"),
        pytest.param(Store.CURRENT, CLOUD_VALUE, None, id="no-marker"),
        pytest.param(Store.CLOUD, CLOUD_VALUE[:9], None, id="truncated-varint"),
    ],
)
def test_reads_value_timestamp(
    store: Store,
    data: bytes,
    expected: int | None,
) -> None:
    assert value_timestamp(store, data) == expected


def test_summarizes_night_light_value_access() -> None:
    events = [
        event(operation="RegOpenKey", data=None),
        event(path="HKCU\\Software\\Other\\Data"),
        event(filetime=START_FILETIME + 5_000, operation="RegQueryValue", data=None),
        event(filetime=START_FILETIME + 25_000),
        event(
            filetime=START_FILETIME + 40_000,
            operation="RegQueryValue",
            path=CLOUD_SETTINGS,
            data=CLOUD_VALUE,
        ),
        event(filetime=START_FILETIME + 55_000, path=CURRENT_STATE),
    ]
    assert list(summarize(events)) == [
        SummaryRow(
            ms=0.0,
            process="explorer.exe",
            pid=4321,
            operation=Operation.WRITE,
            store=Store.CURRENT,
            record=Record.SETTINGS,
            stamp=1792362613,
            length=16,
            utc=datetime(2026, 10, 19, 2, 10, 3, 2500, tzinfo=UTC),
        ),
        SummaryRow(
            ms=1.5,
            process="explorer.exe",
            pid=4321,
            operation=Operation.READ,
            store=Store.CLOUD,
            record=Record.SETTINGS,
            stamp=1792362613,
            length=13,
            utc=datetime(2026, 10, 19, 2, 10, 3, 4000, tzinfo=UTC),
        ),
        SummaryRow(
            ms=3.0,
            process="explorer.exe",
            pid=4321,
            operation=Operation.WRITE,
            store=Store.CURRENT,
            record=Record.STATE,
            stamp=1792362613,
            length=16,
            utc=datetime(2026, 10, 19, 2, 10, 3, 5500, tzinfo=UTC),
        ),
    ]
