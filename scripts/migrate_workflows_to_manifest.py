#!/usr/bin/env python3
"""Convert legacy workflow JSON files to canonical workflow manifests.

This is intentionally mechanical: it preserves the existing engine steps under
manifest `steps[]`, infers parameter and side-effect contracts, and writes the
manifest back to the workflow's existing filename.
"""

from __future__ import annotations

import argparse
import json
import re
from pathlib import Path
from typing import Any


ACTION_KIND_BY_STEP_TYPE = {
    "assert_selector_state": "assert_selector_state",
    "assert_text_in_element": "assert_text_in_element",
    "assert_url_matches": "assert_url_matches",
    "apply_filter_by_text": "apply_filter_by_text",
    "capture_ui_bundle": "capture_ui_bundle",
    "click": "click",
    "click_element": "click_element",
    "clear_cookies": "clear_cookies",
    "clear_enhanced_caches": "clear_enhanced_caches",
    "clear_local_storage": "clear_local_storage",
    "close_current_tab": "close_current_tab",
    "configure_captcha_solver": "configure_captcha_solver",
    "date_set_range": "date_set_range",
    "dbl_click_element": "dbl_click_element",
    "detect_popups": "detect_popups",
    "dismiss_popups": "dismiss_popups",
    "download": "download",
    "download_file": "download_file",
    "download_images": "download_images",
    "drag_and_drop": "drag_and_drop",
    "eval_isolated_world": "eval_isolated_world",
    "eval_main_world": "eval_main_world",
    "execute_javascript": "execute_javascript",
    "execute_extraction_plan": "execute_extraction_plan",
    "extract": "extract",
    "extract_page_assets": "extract_page_assets",
    "extract_structured_data": "extract_structured_data",
    "fill_input_field": "fill_input_field",
    "fill_and_submit": "fill_and_submit",
    "get_cookies": "get_cookies",
    "get_current_url": "get_current_url",
    "get_element_attribute": "get_element_attribute",
    "get_element_count": "get_element_count",
    "get_element_text": "get_element_text",
    "get_element_value": "get_element_value",
    "get_local_storage_item": "get_local_storage_item",
    "get_page_source": "get_page_source",
    "get_performance_stats": "get_performance_stats",
    "handle_captcha": "handle_captcha",
    "hover_element": "hover_element",
    "infinite_scroll": "infinite_scroll",
    "inspect_click_surface": "inspect_click_surface",
    "inspect_element": "inspect_element",
    "navigate": "navigate",
    "navigate_to_url": "navigate_to_url",
    "observe": "observe",
    "open_new_tab": "open_new_tab",
    "press_key": "press_key",
    "press_special_key": "press_special_key",
    "read_field_value": "read_field_value",
    "request_user_intervention": "request_user_intervention",
    "same_origin_request": "same_origin_request",
    "scroll": "scroll",
    "scroll_element_into_view": "scroll_element_into_view",
    "scroll_window_to": "scroll_window_to",
    "select_result": "select_result",
    "select_option": "select_option",
    "select_option_in_dropdown": "select_option_in_dropdown",
    "semantic_action": "semantic_action",
    "set_cookie": "set_cookie",
    "set_local_storage_item": "set_local_storage_item",
    "simulate_human_behavior": "simulate_human_behavior",
    "submit_input": "submit_input",
    "submit_text_query": "submit_text_query",
    "switch_to_tab": "switch_to_tab",
    "take_screenshot": "take_screenshot",
    "type_text": "type_text",
    "upload_file": "upload_file",
    "verify_ui_change": "verify_ui_change",
    "wait": "wait",
    "wait_for_auth": "wait_for_auth",
    "wait_for_element": "wait_for_element",
    "wait_for_navigation": "wait_for_navigation",
    "wait_for_network_idle": "wait_for_network_idle",
    "wait_for_no_popups": "wait_for_no_popups",
    "wait_for_timeout": "wait_for_timeout",
    "wait_for_totp": "wait_for_totp",
    "wait_for_verification": "wait_for_verification",
}

TARGET_KEYS = {"encoded_id", "selector", "text", "role", "frame_id"}
STEP_META_KEYS = {
    "id",
    "name",
    "type",
    "timeout_ms",
    "timeoutMs",
    "continue_on_error",
    "retry",
}
PLACEHOLDER_RE = re.compile(r"\{([A-Za-z_][A-Za-z0-9_]*)\}")


def slugify(value: str) -> str:
    value = re.sub(r"[^A-Za-z0-9]+", "_", value.strip().lower())
    return value.strip("_")


def workflow_name_from_stem(system: str, stem: str) -> str:
    stem = slugify(stem)
    prefix = f"{system}_"
    if stem.startswith(prefix):
        stem = stem[len(prefix) :]
    return stem


def iter_workflow_files(root: Path) -> list[Path]:
    files = []
    for path in root.rglob("*.json"):
        rel = path.relative_to(root)
        if "fixtures" in rel.parts:
            continue
        if ".manifest." in path.name:
            continue
        if any(part.startswith("test") for part in rel.parts):
            continue
        files.append(path)
    return sorted(files)


def read_json(path: Path) -> dict[str, Any] | None:
    try:
        value = json.loads(path.read_text())
    except Exception:
        return None
    return value if isinstance(value, dict) else None


def sequence(workflow: dict[str, Any]) -> dict[str, Any]:
    sequences = workflow.get("browser_automation", {}).get("sequences", [])
    if sequences and isinstance(sequences[0], dict):
        return sequences[0]
    return {}


def help_params(workflow: dict[str, Any]) -> dict[str, dict[str, Any]]:
    out: dict[str, dict[str, Any]] = {}
    params = workflow.get("help", {}).get("parameters", [])
    if isinstance(params, list):
        for item in params:
            if isinstance(item, dict) and item.get("name"):
                out[str(item["name"])] = item
    elif isinstance(params, dict):
        for name, description in params.items():
            out[str(name)] = {"name": str(name), "description": str(description)}
    return out


def collect_placeholders(value: Any, out: set[str], parent_key: str | None = None) -> None:
    if isinstance(value, str):
        if parent_key == "script":
            return
        out.update(match.group(1) for match in PLACEHOLDER_RE.finditer(value))
    elif isinstance(value, list):
        for item in value:
            collect_placeholders(item, out, parent_key)
    elif isinstance(value, dict):
        for key, child in value.items():
            collect_placeholders(child, out, str(key))


def param_kind(name: str, shape: str | None) -> str:
    text = f"{name} {shape or ''}".lower()
    if "bool" in text:
        return "boolean"
    if "integer" in text or name.endswith("_count") or name.startswith("max_"):
        return "integer"
    if "number" in text:
        return "number"
    if "array" in text or "list" in text:
        return "array"
    if "object" in text or "json" in text:
        return "object"
    return "string"


def coerce_default(value: Any, kind: str) -> Any:
    if value is None:
        return None
    if kind == "integer":
        try:
            return int(str(value).strip())
        except ValueError:
            return None
    if kind == "number":
        try:
            return float(str(value).strip())
        except ValueError:
            return None
    if kind == "boolean":
        if isinstance(value, bool):
            return value
        text = str(value).strip().lower()
        if text in {"1", "true", "yes", "on"}:
            return True
        if text in {"0", "false", "no", "off"}:
            return False
        return None
    return value


def build_params(workflow: dict[str, Any]) -> dict[str, Any]:
    seq = sequence(workflow)
    help_by_name = help_params(workflow)
    required = {
        str(item.get("name", "")).strip()
        for item in seq.get("required_variables", [])
        if isinstance(item, dict) and item.get("name")
    }
    optional = {
        str(item.get("name", "")).strip()
        for item in seq.get("optional_variables", [])
        if isinstance(item, dict) and item.get("name")
    }
    placeholders: set[str] = set()
    collect_placeholders(workflow, placeholders)
    names = sorted((required | optional | placeholders | set(help_by_name)) - {""})
    properties: dict[str, Any] = {}
    for name in names:
        meta = help_by_name.get(name, {})
        description = meta.get("description")
        shape = meta.get("shape")
        kind = param_kind(name, shape)
        item: dict[str, Any] = {
            "kind": kind,
            "required": name in required or bool(meta.get("required")),
            "description": description or f"Value for `{name}`.",
        }
        if meta.get("sensitive"):
            item["sensitive"] = True
        enum_values = []
        if isinstance(shape, str) and shape.startswith("enum:"):
            enum_values = [part.strip() for part in shape[5:].split("|") if part.strip()]
            item["enum_values"] = enum_values
        default = coerce_default(meta.get("default"), kind)
        if default is not None and (not enum_values or default in enum_values):
            item["default"] = default
        if isinstance(shape, str) and shape.startswith("enum:"):
            item["enum_values"] = enum_values
        properties[name] = item
    return {"properties": properties, "additional_params": False}


def side_effects_for_step_type(step_type: str) -> list[str]:
    if step_type in {
        "get_page_source",
        "get_current_url",
        "get_element_text",
        "get_element_attribute",
        "get_element_value",
        "get_element_count",
        "read_field_value",
        "observe",
        "wait",
        "wait_for_element",
        "wait_for_timeout",
        "wait_for_navigation",
        "wait_for_network_idle",
        "wait_for_no_popups",
        "wait_for_auth",
        "wait_for_totp",
        "wait_for_verification",
        "extract",
        "extract_structured_data",
        "extract_page_assets",
        "execute_extraction_plan",
        "assert_selector_state",
        "assert_text_in_element",
        "assert_url_matches",
        "take_screenshot",
        "capture_ui_bundle",
        "inspect_element",
        "inspect_click_surface",
        "verify_ui_change",
        "get_performance_stats",
    }:
        return ["external_read", "read_only"]
    if step_type == "same_origin_request":
        return ["external_read", "network_access", "read_only"]
    if step_type in {"download", "download_file", "download_images"}:
        return ["browser_state", "download", "external_read", "file_write", "network_access"]
    if step_type in {"navigate", "navigate_to_url", "open_new_tab", "switch_to_tab"}:
        return ["browser_state", "external_read", "network_access"]
    if step_type in {"upload_file", "submit_input", "fill_and_submit", "submit_text_query"}:
        return (
            ["browser_state"]
            if step_type == "submit_input"
            else ["browser_state", "external_write", "network_access"]
        )
    if step_type in {"get_cookies", "get_local_storage_item"}:
        return ["browser_state", "read_only"]
    if step_type in {"clear_cookies", "set_cookie", "clear_local_storage", "set_local_storage_item"}:
        return ["browser_state"]
    return ["browser_state"]


def build_side_effect_declarations(classes: set[str], workflow: dict[str, Any]) -> list[dict[str, Any]]:
    domain = workflow.get("domain")
    scopes = [domain] if isinstance(domain, str) and domain else []
    return [
        {
            "class": klass,
            "idempotency": "safe_retry",
            "confirmation_required": klass in {"external_write", "destructive"},
            "scopes": scopes if klass != "file_write" else [],
        }
        for klass in sorted(classes)
    ]


def workflow_requires_external_write(system: str, workflow_name: str, workflow: dict[str, Any]) -> bool:
    name_text = f"{system}_{workflow_name}".lower()
    mutating_tokens = {
        "create",
        "send",
        "reply",
        "submit",
        "comment",
        "vote",
        "follow",
        "like",
    }
    if any(token in name_text.split("_") for token in mutating_tokens):
        return True
    if any(token in name_text for token in {"send_dm", "reply_dm", "dm_send", "create_post"}):
        return True
    description = " ".join(
        str(workflow.get(key) or "")
        for key in ("name", "description")
    ).lower()
    return any(
        phrase in description
        for phrase in (
            "send a direct message",
            "send another prompt",
            "submit a reddit post",
            "submit a link",
            "create a post",
            "publish",
        )
    )


def step_external_write_candidate(step: dict[str, Any]) -> bool:
    step_type = str(step.get("type", ""))
    if step_type in {
        "click_element",
        "dbl_click_element",
        "drag_and_drop",
        "execute_javascript",
        "eval_main_world",
        "eval_isolated_world",
        "fill_and_submit",
        "submit_input",
        "submit_text_query",
        "type_text",
        "fill_input_field",
        "press_key",
        "press_special_key",
        "upload_file",
    }:
        return True
    return False


def convert_step(step: dict[str, Any], workflow_external_write: bool) -> tuple[dict[str, Any], set[str]]:
    step_type = str(step.get("type", "custom"))
    kind = ACTION_KIND_BY_STEP_TYPE.get(step_type, "custom")
    effects = set(side_effects_for_step_type(step_type))
    if workflow_external_write and step_external_write_candidate(step):
        effects.add("external_write")
        effects.add("network_access")
    target = {key: step[key] for key in TARGET_KEYS if key in step}
    inputs = {
        key: value
        for key, value in step.items()
        if key not in STEP_META_KEYS and key not in TARGET_KEYS
    }
    action: dict[str, Any] = {
        "kind": kind,
        "inputs": inputs,
        "side_effects": sorted(effects),
    }
    if kind == "custom":
        action["custom_kind"] = step_type
    if target:
        action["target"] = target
    converted: dict[str, Any] = {
        "id": str(step.get("id") or f"step_{abs(hash(json.dumps(step, sort_keys=True))) % 100000}"),
        "action": action,
    }
    if step.get("name"):
        converted["name"] = step["name"]
    timeout = step.get("timeout_ms", step.get("timeoutMs"))
    if isinstance(timeout, int):
        converted["timeout_ms"] = timeout
    if step.get("continue_on_error"):
        converted["continue_on_error"] = True
    return converted, effects


def output_schema_for_step(step: dict[str, Any] | None) -> dict[str, Any] | None:
    if not step:
        return None
    step_type = step.get("type")
    if step_type == "extract_structured_data":
        props: dict[str, Any] = {}
        for field in step.get("fields", []):
            if isinstance(field, dict) and field.get("name"):
                props[str(field["name"])] = {"type": "string"}
        return {"type": "array", "items": {"type": "object", "properties": props}}
    if step_type == "get_element_text":
        return {"type": "string"}
    if step_type == "download_images":
        return {
            "type": "object",
            "properties": {
                "image_urls": {"type": "array", "items": {"type": "string", "format": "uri"}},
                "downloads": {"type": "array", "items": {"type": "object"}},
            },
        }
    if step_type == "execute_javascript":
        return {"type": "object"}
    return None


def result_step(steps: list[dict[str, Any]]) -> dict[str, Any] | None:
    for preferred_type in ("extract_structured_data", "download_images", "execute_javascript", "get_element_text"):
        for step in reversed(steps):
            if step.get("type") == preferred_type:
                return step
    return None


def build_manifest(path: Path, root: Path, workflow: dict[str, Any]) -> dict[str, Any] | None:
    rel = path.relative_to(root)
    if len(rel.parts) < 2:
        return None
    system = slugify(rel.parts[0])
    workflow_name = workflow_name_from_stem(system, path.stem)
    external_write = workflow_requires_external_write(system, workflow_name, workflow)
    seq = sequence(workflow)
    legacy_steps = [step for step in seq.get("steps", []) if isinstance(step, dict)]
    converted_steps = []
    all_effects: set[str] = set()
    for step in legacy_steps:
        converted, effects = convert_step(step, external_write)
        converted_steps.append(converted)
        all_effects.update(effects)
    final_step = result_step(legacy_steps)
    result: dict[str, Any] = {
        "artifact_policy": {"prefer_downloads": "download" in all_effects},
        "include_debug": False,
    }
    schema = output_schema_for_step(final_step)
    if schema:
        result["output_schema"] = schema
    if final_step and final_step.get("id"):
        result["output_selector"] = {"step_id": str(final_step["id"]), "path": "$"}
    returns = workflow.get("help", {}).get("returns")
    help_block = {
        "summary": workflow.get("help", {}).get("summary")
        or workflow.get("description")
        or workflow.get("name")
        or f"{system}/{workflow_name}",
        "parameters": {
            name: item.get("description", f"Value for `{name}`.")
            for name, item in build_params(workflow)["properties"].items()
        },
        "examples": workflow.get("help", {}).get("examples", []),
        "returns": [returns] if isinstance(returns, str) and returns else [],
        "notes": workflow.get("help", {}).get("notes", []),
    }
    return {
        "schema_version": "rzn.workflow_manifest",
        "id": f"{system}/{workflow_name}",
        "name": workflow.get("name") or f"{system}/{workflow_name}",
        "version": str(workflow.get("version") or "1.0.0"),
        "system": system,
        "capability": f"{system}.{workflow_name.replace('_', '.')}",
        "summary": workflow.get("help", {}).get("summary") or workflow.get("description"),
        "description": workflow.get("description"),
        "params": build_params(workflow),
        "side_effects": build_side_effect_declarations(all_effects or {"read_only"}, workflow),
        "runtime": {
            "actor": "supervisor",
            "requires_cdp": False,
            "requires_existing_session": bool(
                workflow.get("browser_automation", {}).get("use_current_tab")
                or workflow.get("browser_automation", {}).get("use_active_tab")
            ),
            "timeout_ms": 90000,
        },
        "steps": converted_steps,
        "result": result,
        "help": help_block,
        "metadata": {
            "migrated_from": str(rel),
            "legacy_system_id": workflow.get("system_id"),
        },
    }


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--root", default="workflows", type=Path)
    parser.add_argument("--write", action="store_true")
    parser.add_argument("--force", action="store_true")
    args = parser.parse_args()

    root = args.root
    generated = 0
    skipped = 0
    for path in iter_workflow_files(root):
        workflow = read_json(path)
        if not workflow or "browser_automation" not in workflow:
            continue
        manifest = build_manifest(path, root, workflow)
        if not manifest:
            continue
        dest = path
        if dest.exists() and not args.force:
            skipped += 1
            continue
        generated += 1
        if args.write:
            dest.write_text(json.dumps(manifest, indent=2) + "\n")
            print(f"wrote {dest}")
        else:
            print(f"would write {dest}")
    print(json.dumps({"generated": generated, "skipped_existing": skipped, "write": args.write}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
