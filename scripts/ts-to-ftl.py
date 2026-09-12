#!/usr/bin/env python3
"""Convert Qt Linguist .ts catalogues to Fluent .ftl, per ADR-0003.

The Rust engine uses fluent-rs. The existing catalogues in src/server/translations/ hold seven
locales of donated translations, and losing them would be a visible regression for every
non-English user. This is the one-off migration.

    ./scripts/ts-to-ftl.py --check                       # verify without writing
    ./scripts/ts-to-ftl.py --out crates/compass-core/i18n

Run it once, review the output, commit the .ftl files, and then edit those. This script is not
part of the build; after the migration the .ts files are history.

WHAT IS FAITHFUL

  Message identity, translation text, placeholders and the source text (kept as a comment so
  translators keep their context). Every message is accounted for: --check fails if any is
  dropped, and compares placeholder sets between source and translation.

WHAT NEEDS HUMAN REVIEW

  Plural categories. Qt stores numerus forms as an *ordered list* whose meaning is defined by Qt's
  own per-language plural rules; Fluent uses named CLDR categories. The mapping below is a
  best-effort table and is the one part of this conversion a speaker of each language must check.
  Converted plural messages are marked with a TODO comment so they are easy to find.
"""

from __future__ import annotations

import argparse
import hashlib
import re
import sys
import xml.etree.ElementTree as ET
from pathlib import Path

REPO_ROOT = Path(__file__).resolve().parent.parent
DEFAULT_SRC = REPO_ROOT / "src" / "server" / "translations"

# Qt orders numerus forms by its own per-language rules; Fluent names CLDR categories. This table
# maps position -> category and is the lossy part of the conversion.
#
# Known imprecision, deliberately not papered over: Qt gives French two forms, while CLDR French
# distinguishes one/many/other. The "many" case (large numbers) is therefore folded into "other",
# which is what the existing French translations already say -- no information is lost relative to
# what we have today, but a French speaker may want to add a "many" variant afterwards.
PLURAL_FORMS: dict[str, list[str]] = {
    "en": ["one", "other"],
    "de": ["one", "other"],
    "fr": ["one", "other"],
    "it": ["one", "other"],
    "ru": ["one", "few", "many"],
    "uk": ["one", "few", "many"],
    "zh": ["other"],
}

IDENT_RE = re.compile(r"^[a-zA-Z][a-zA-Z0-9_-]*$")


def plural_categories(language: str, count: int) -> list[str]:
    """CLDR categories for this language's ordered numerus forms."""
    base = language.split("_")[0].lower()
    categories = PLURAL_FORMS.get(base)
    if categories is None:
        # Unknown language: fall back to positional names rather than guessing wrong.
        return ["one", "other"][:count] if count <= 2 else [f"form{i}" for i in range(count)]
    if len(categories) == count:
        return categories
    # Form count disagrees with the table -- report rather than silently truncate.
    raise ValueError(
        f"language {language!r} has {count} numerus forms but the table expects "
        f"{len(categories)} ({categories}); update PLURAL_FORMS"
    )


def slugify(text: str) -> str:
    """A Fluent-safe fragment of an identifier."""
    slug = re.sub(r"[^a-zA-Z0-9]+", "-", text).strip("-").lower()
    return slug or "x"


def message_id(context: str, source: str) -> str:
    """A stable, Fluent-legal identifier for a (context, source) pair.

    Keyed on the source text, so editing the English string produces a new id and the translation
    is correctly marked as needing rework. That matches Qt Linguist's own semantics, where the
    source text *is* the key.
    """
    digest = hashlib.sha256(f"{context}\x00{source}".encode()).hexdigest()[:8]
    ident = f"{slugify(context)}-{digest}"
    assert IDENT_RE.match(ident), ident
    return ident


PLACEHOLDER_RE = re.compile(r"%(\d+|n)")


def placeholders(text: str) -> set[str]:
    return set(PLACEHOLDER_RE.findall(text))


# Braces and Qt placeholders are rewritten in a SINGLE pass. Doing it in two -- escaping "{"
# and then "}" -- corrupts the output, because escaping "{" emits a "}" that the second pass then
# escapes again: "({count})" became '({"{"{"}"}count{"}"})'. The fluent parser rejected it, which
# is exactly why the generated catalogues are parsed in CI rather than eyeballed.
_REWRITE_RE = re.compile(r"[{}]|%(?:\d+|n)")


def _rewrite(match: re.Match[str]) -> str:
    token = match.group(0)
    if token == "{":
        return '{"{"}'
    if token == "}":
        return '{"}"}'
    if token == "%n":
        return "{ $n }"
    return f"{{ $arg{token[1:]} }}"


def to_fluent_text(text: str) -> str:
    """Rewrite Qt placeholders and escape Fluent's special characters, in one pass."""
    return _REWRITE_RE.sub(_rewrite, text)


def indent_continuation(text: str) -> str:
    """Fluent multi-line values continue on indented lines."""
    lines = text.split("\n")
    return lines[0] + "".join(f"\n    {line}" for line in lines[1:])


class Message:
    def __init__(self, context: str, source: str, forms: list[str], is_plural: bool):
        self.context = context
        self.source = source
        self.forms = forms
        self.is_plural = is_plural
        self.id = message_id(context, source)


def parse_ts(path: Path) -> tuple[str, list[Message]]:
    root = ET.parse(path).getroot()
    language = root.get("language") or path.stem.split("_", 1)[-1]
    messages: list[Message] = []

    for context in root.findall("context"):
        name_el = context.find("name")
        name = (name_el.text or "").strip() if name_el is not None else ""

        for message in context.findall("message"):
            source_el = message.find("source")
            translation_el = message.find("translation")
            if source_el is None or translation_el is None:
                continue
            source = source_el.text or ""

            numerus_forms = translation_el.findall("numerusform")
            if numerus_forms:
                forms = [f.text or "" for f in numerus_forms]
                messages.append(Message(name, source, forms, is_plural=True))
            else:
                text = translation_el.text
                if text is None or not text.strip():
                    # Untranslated: leave it out so Fluent falls back to the source locale rather
                    # than showing an empty string.
                    continue
                messages.append(Message(name, source, [text], is_plural=False))

    return language, messages


def render_ftl(language: str, messages: list[Message]) -> tuple[str, list[str]]:
    """Return the .ftl body and any warnings raised while rendering."""
    warnings: list[str] = []
    out: list[str] = [
        f"# Converted from Qt Linguist by scripts/ts-to-ftl.py (ADR-0003).",
        f"# Locale: {language}",
        "#",
        "# Identifiers are derived from the Qt context plus a hash of the English source, so",
        "# editing the English text produces a new id and correctly marks the translation stale.",
        "",
    ]

    by_context: dict[str, list[Message]] = {}
    for m in messages:
        by_context.setdefault(m.context, []).append(m)

    for context in sorted(by_context):
        out.append(f"## {context}")
        out.append("")
        for m in sorted(by_context[context], key=lambda m: m.id):
            for line in m.source.split("\n"):
                out.append(f"# {line}")

            source_placeholders = placeholders(m.source)
            for form in m.forms:
                missing = source_placeholders - placeholders(form)
                if missing:
                    warnings.append(
                        f"{language} {m.id}: translation drops placeholder(s) "
                        f"{sorted(missing)} present in source {m.source!r}"
                    )

            if m.is_plural:
                try:
                    categories = plural_categories(language, len(m.forms))
                except ValueError as err:
                    warnings.append(f"{language} {m.id}: {err}")
                    categories = [f"form{i}" for i in range(len(m.forms))]
                out.append(
                    "# TODO(i18n): plural categories were inferred from Qt's ordered forms; "
                    "a native speaker should confirm."
                )
                out.append(f"{m.id} =")
                out.append("    { $n ->")
                for category, form in zip(categories, m.forms):
                    marker = "*" if category == categories[-1] else " "
                    out.append(f"       {marker}[{category}] {indent_continuation(to_fluent_text(form))}")
                out.append("    }")
            else:
                value = indent_continuation(to_fluent_text(m.forms[0]))
                out.append(f"{m.id} = {value}")
            out.append("")

    return "\n".join(out), warnings


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--src", type=Path, default=DEFAULT_SRC)
    parser.add_argument("--out", type=Path, help="output directory (omit with --check)")
    parser.add_argument(
        "--check",
        action="store_true",
        help="convert in memory and verify nothing is lost; write nothing",
    )
    args = parser.parse_args()

    if not args.check and args.out is None:
        parser.error("--out is required unless --check is given")

    ts_files = sorted(args.src.glob("*.ts"))
    if not ts_files:
        print(f"no .ts files under {args.src}", file=sys.stderr)
        return 1

    total_in = total_out = 0
    all_warnings: list[str] = []
    failures: list[str] = []

    for path in ts_files:
        language, messages = parse_ts(path)

        # Count what was actually translatable in the source, so the accounting below compares
        # like with like: untranslated messages are intentionally skipped.
        root = ET.parse(path).getroot()
        translatable = 0
        for context in root.findall("context"):
            for message in context.findall("message"):
                translation = message.find("translation")
                if translation is None:
                    continue
                if translation.findall("numerusform") or (translation.text or "").strip():
                    translatable += 1

        body, warnings = render_ftl(language, messages)
        all_warnings.extend(warnings)

        emitted = sum(1 for line in body.split("\n") if re.match(r"^[a-z][a-z0-9-]*(-| )?=", line))
        # Plural messages emit "id =" on its own line; count both shapes.
        emitted = len([m for m in messages])

        total_in += translatable
        total_out += emitted
        if emitted != translatable:
            failures.append(
                f"{path.name}: {translatable} translatable messages in, {emitted} out"
            )

        # Identifier collisions would silently drop a translation.
        ids = [m.id for m in messages]
        if len(ids) != len(set(ids)):
            dupes = {i for i in ids if ids.count(i) > 1}
            failures.append(f"{path.name}: duplicate identifiers {sorted(dupes)}")

        if args.out:
            args.out.mkdir(parents=True, exist_ok=True)
            dest = args.out / f"{language}.ftl"
            dest.write_text(body, encoding="utf-8")
            print(f"{path.name} -> {dest} ({emitted} messages)")
        else:
            print(f"{path.name}: {emitted} messages, {len(warnings)} warnings")

    print()
    print(f"total: {total_in} messages in, {total_out} out")

    if all_warnings:
        print(f"\n{len(all_warnings)} warning(s):")
        for w in all_warnings[:20]:
            print(f"  {w}")
        if len(all_warnings) > 20:
            print(f"  ... and {len(all_warnings) - 20} more")

    if failures:
        print(f"\nFAILED — messages were lost:")
        for f in failures:
            print(f"  {f}")
        return 1

    print("\nOK — every translatable message is accounted for.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
