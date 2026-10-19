import io

from procmon_parser import Column, RuleAction, RuleRelation, load_configuration

from rewind_trace.procmon import write_cloudstore_filter


def test_filter_keeps_only_cloudstore_events() -> None:
    config = io.BytesIO()
    write_cloudstore_filter(config)
    config.seek(0)
    loaded = load_configuration(config)
    assert loaded["DestructiveFilter"] == 1
    assert [
        (rule.column, rule.relation, rule.value, rule.action)
        for rule in loaded["FilterRules"]
    ] == [
        (Column.PATH, RuleRelation.CONTAINS, "CloudStore", RuleAction.INCLUDE),
        (Column.PROCESS_NAME, RuleRelation.IS, "Procmon.exe", RuleAction.EXCLUDE),
        (Column.PROCESS_NAME, RuleRelation.IS, "Procmon64.exe", RuleAction.EXCLUDE),
    ]
