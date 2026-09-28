#!/usr/bin/env python3
"""Tests for llm_providers.py slot parsing and structured-output fallback (standard library only)."""

from __future__ import annotations

import contextlib
import io
import unittest

import llm_providers
from llm_providers import ProviderSlot, _RetryWithoutFormat

SCHEMA = {"type": "object", "properties": {}}


def slot(protocol: str = "openai", response_format: str = "auto") -> ProviderSlot:
    return ProviderSlot(2, protocol, "https://example.invalid", "model", "key", response_format)


class LoadSlotsTest(unittest.TestCase):
    def test_response_format_defaults_to_auto(self) -> None:
        slots, problems = llm_providers.load_slots({
            "RELEASE_NOTES_LLM_2_API_KEY": "k",
            "RELEASE_NOTES_LLM_2_PROTOCOL": "openai",
            "RELEASE_NOTES_LLM_2_MODEL": "deepseek-flash",
        })
        self.assertEqual(problems, [])
        self.assertEqual([s.response_format for s in slots], ["auto"])

    def test_response_format_is_read_case_insensitively(self) -> None:
        slots, _ = llm_providers.load_slots({
            "RELEASE_NOTES_LLM_1_API_KEY": "k",
            "RELEASE_NOTES_LLM_1_PROTOCOL": "openai",
            "RELEASE_NOTES_LLM_1_MODEL": "m",
            "RELEASE_NOTES_LLM_1_RESPONSE_FORMAT": " None ",
        })
        self.assertEqual(slots[0].response_format, "none")

    def test_inapplicable_format_is_reported_and_falls_back_to_auto(self) -> None:
        slots, problems = llm_providers.load_slots({
            "RELEASE_NOTES_LLM_1_API_KEY": "k",
            "RELEASE_NOTES_LLM_1_RESPONSE_FORMAT": "json_object",
        })
        self.assertEqual(slots[0].protocol, "anthropic")
        self.assertEqual(slots[0].response_format, "auto")
        self.assertEqual(len(problems), 1)
        self.assertIn("RESPONSE_FORMAT", problems[0])


class FormatChainTest(unittest.TestCase):
    def test_openai_auto_tries_schema_then_object_then_plain(self) -> None:
        chain = llm_providers.format_chain(slot(), SCHEMA)
        self.assertEqual([c and c["type"] for c in chain], ["json_schema", "json_object", None])

    def test_openai_json_object_skips_schema(self) -> None:
        chain = llm_providers.format_chain(slot(response_format="json_object"), SCHEMA)
        self.assertEqual([c and c["type"] for c in chain], ["json_object", None])

    def test_none_sends_no_format_parameter(self) -> None:
        self.assertEqual(llm_providers.format_chain(slot(response_format="none"), SCHEMA), [None])
        self.assertEqual(llm_providers.format_chain(slot("anthropic", "none"), SCHEMA), [None])

    def test_anthropic_auto_tries_schema_then_plain(self) -> None:
        self.assertEqual(llm_providers.format_chain(slot("anthropic"), SCHEMA), [SCHEMA, None])


class CompleteFallbackTest(unittest.TestCase):
    def test_none_makes_exactly_one_request_without_format(self) -> None:
        calls: list = []

        def fake(slot_, system, messages, option):
            calls.append(option)
            return "{}"

        original = llm_providers._openai_complete
        llm_providers._openai_complete = fake
        try:
            self.assertEqual(llm_providers._complete(slot(response_format="none"), "s", [], SCHEMA), "{}")
        finally:
            llm_providers._openai_complete = original
        self.assertEqual(calls, [None])

    def test_auto_degrades_until_a_format_is_accepted(self) -> None:
        calls: list = []

        def fake(slot_, system, messages, option):
            calls.append(option and option["type"])
            if option is not None:
                raise _RetryWithoutFormat("This response_format type is unavailable now")
            return "{}"

        original = llm_providers._openai_complete
        llm_providers._openai_complete = fake
        try:
            with contextlib.redirect_stderr(io.StringIO()):
                llm_providers._complete(slot(), "s", [], SCHEMA)
        finally:
            llm_providers._openai_complete = original
        self.assertEqual(calls, ["json_schema", "json_object", None])


if __name__ == "__main__":
    unittest.main()
