//! The browser side of `rzn.item/v2`: the vendored rzn-tools schemas (repo `schema/`) plus the
//! rules of rzn-tools `item::check()` that a JSON schema cannot express. The fleet server runs the
//! real `check()` on every result; this keeps recorded samples and authoring honest before that.

use jsonschema::{Draft, JSONSchema};
use rzn_contracts::workflow::OutputContract;
use serde_json::Value;
use std::collections::HashSet;

pub const ITEM_SCHEMA: &str = include_str!("../../../schema/rzn.item.v2.json");
pub const ITEMS_PAGE_SCHEMA: &str = include_str!("../../../schema/rzn.items.v2.json");
pub const DESCRIBE_SCHEMA: &str = include_str!("../../../schema/rzn.describe.v2.json");

const ITEM_TYPE: &str = "rzn.item/v2";
const ITEMS_TYPE: &str = "rzn.items/v2";
const DESCRIBE_TYPE: &str = "rzn.describe/v2";

/// Checks a workflow output against its declared contract. `none` accepts anything.
pub fn check_output(contract: OutputContract, value: &Value) -> Result<(), Vec<String>> {
    match contract {
        OutputContract::ItemV2 => check_item(value),
        OutputContract::ItemsV2 => check_items_page(value),
        OutputContract::None => Ok(()),
    }
}

/// A single `rzn.item/v2` result: always a full item.
pub fn check_item(value: &Value) -> Result<(), Vec<String>> {
    let mut errors = schema_errors(ITEM_SCHEMA, value);
    if errors.is_empty() {
        if let Err(error) = full_item_rules(value) {
            errors.push(error);
        }
    }
    into_result(errors)
}

/// A `rzn.items/v2` page. Every entry passes the listing checks; an entry that carries a body
/// (blocks or a document) and is not marked truncated must also be a full item.
pub fn check_items_page(value: &Value) -> Result<(), Vec<String>> {
    let mut errors = schema_errors(ITEMS_PAGE_SCHEMA, value);
    if !errors.is_empty() {
        return Err(errors);
    }
    if value.get("type").and_then(Value::as_str) != Some(ITEMS_TYPE) {
        errors.push(format!("type must be {ITEMS_TYPE}"));
    }
    for (index, item) in value["items"].as_array().into_iter().flatten().enumerate() {
        let name = item["key"].as_str().unwrap_or("?");
        let has_body =
            item["blocks"].as_array().is_some_and(|b| !b.is_empty()) || !item["document"].is_null();
        let truncated = !item["truncated"].is_null();
        let checked = if has_body && !truncated {
            full_item_rules(item)
        } else {
            listed_rules(item)
        };
        if let Err(error) = checked {
            errors.push(format!("items[{index}] {name}: {error}"));
        }
    }
    into_result(errors)
}

/// A family's `rzn.describe/v2`.
pub fn check_describe(value: &Value) -> Result<(), Vec<String>> {
    let mut errors = schema_errors(DESCRIBE_SCHEMA, value);
    if !errors.is_empty() {
        return Err(errors);
    }
    if value.get("type").and_then(Value::as_str) != Some(DESCRIBE_TYPE) {
        errors.push(format!("type must be {DESCRIBE_TYPE}"));
    }
    if value["connector"]
        .as_str()
        .is_some_and(|c| c.trim().is_empty())
    {
        errors.push("connector is empty".to_string());
    }
    let kinds = value["kinds"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    if kinds.is_empty() {
        errors.push("kinds is empty".to_string());
    }
    for kind in kinds {
        let name = kind["kind"].as_str().unwrap_or("");
        if let Err(error) = check_key_shape(kind["identity"].as_str().unwrap_or(""), true) {
            errors.push(format!("kind {name}: identity {error}"));
        }
        let mut seen = HashSet::new();
        for field in kind["fields"].as_array().into_iter().flatten() {
            let field_name = field["name"].as_str().unwrap_or("");
            if !seen.insert(field_name) {
                errors.push(format!("kind {name}: field {field_name} repeats"));
            }
        }
    }
    into_result(errors)
}

/// `<site>:<kind>:<id>`: a lowercase site, a lowercase kind, then a non-empty id without spaces.
pub fn check_key(key: &str) -> Result<(), String> {
    check_key_shape(key, false)
}

/// `placeholder` allows an identity rule such as `x:post:<first post id>`.
fn check_key_shape(key: &str, placeholder: bool) -> Result<(), String> {
    let mut parts = key.splitn(3, ':');
    let site = parts.next().unwrap_or("");
    let kind = parts.next().unwrap_or("");
    let id = parts.next().unwrap_or("");
    let word = |part: &str, extra: &[u8]| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || extra.contains(&b))
    };
    if !word(site, &b"-."[..])
        || !word(kind, &b"_"[..])
        || id.is_empty()
        || (!placeholder && id.contains(char::is_whitespace))
    {
        return Err(format!("{key:?} is not <site>:<kind>:<id>"));
    }
    Ok(())
}

/// What a listing entry must carry, as `Item::check_listed`, plus the key format.
fn listed_rules(item: &Value) -> Result<(), String> {
    if item.get("type").and_then(Value::as_str) != Some(ITEM_TYPE) {
        return Err(format!("type must be {ITEM_TYPE}"));
    }
    for name in ["key", "kind", "url"] {
        if item[name].as_str().unwrap_or("").trim().is_empty() {
            return Err(format!("{name} is empty"));
        }
    }
    check_key(item["key"].as_str().unwrap_or(""))?;
    if let Some(date) = item["published_at"].as_str() {
        check_date(date)?;
    }
    check_fields(&item["fields"])
}

/// A full item, as `Item::check`.
fn full_item_rules(item: &Value) -> Result<(), String> {
    listed_rules(item)?;
    for name in ["author", "published_in"] {
        if item[name]["name"].as_str().unwrap_or("").trim().is_empty() {
            return Err(format!("{name} is missing"));
        }
    }
    if item["published_at"].as_str().is_none() {
        return Err("published_at is missing".to_string());
    }
    let blocks = item["blocks"].as_array().map(Vec::as_slice).unwrap_or(&[]);
    match (blocks.is_empty(), !item["document"].is_null()) {
        (false, true) => return Err("an item has blocks or a document, not both".to_string()),
        (true, false) => return Err("an item needs blocks or a document".to_string()),
        _ => {}
    }
    if !item["truncated"].is_null() {
        let reason = item["truncated"]["reason"].as_str().unwrap_or("");
        return Err(format!("item is truncated: {reason}"));
    }
    let mut seen = HashSet::new();
    for block in blocks {
        let id = block["id"].as_str().unwrap_or("");
        if id.trim().is_empty() {
            return Err("a block has an empty id".to_string());
        }
        if block["kind"].as_str().unwrap_or("").trim().is_empty() {
            return Err(format!("block {id} has no kind"));
        }
        if let Some(parent) = block["parent"].as_str() {
            if !seen.contains(parent) {
                return Err(format!(
                    "block {id} names parent {parent}, which is not an earlier block"
                ));
            }
        }
        if !seen.insert(id) {
            return Err(format!("block id {id} repeats"));
        }
        let at = &block["at"];
        let span = |range: &Value, start: &str, end: &str| {
            range[end].as_u64().unwrap_or(0) < range[start].as_u64().unwrap_or(0)
        };
        if (!at["time"].is_null() && span(&at["time"], "start_ms", "end_ms"))
            || (!at["chars"].is_null() && span(&at["chars"], "start", "end"))
        {
            return Err(format!("block {id} ends before it starts"));
        }
        if at["time"].is_null() && block["kind"] == "transcript" {
            return Err(format!("transcript block {id} has no time range"));
        }
        if let Some(date) = block["published_at"].as_str() {
            check_date(date).map_err(|error| format!("block {id}: {error}"))?;
        }
        check_fields(&block["fields"]).map_err(|error| format!("block {id}: {error}"))?;
    }
    Ok(())
}

fn check_fields(fields: &Value) -> Result<(), String> {
    let scalar = |v: &Value| v.is_string() || v.is_number() || v.is_boolean();
    for (name, value) in fields.as_object().into_iter().flatten() {
        let ok = scalar(value)
            || value
                .as_array()
                .is_some_and(|values| values.iter().all(scalar));
        if !ok {
            return Err(format!(
                "field {name} is not a string, number, boolean or list of them"
            ));
        }
    }
    Ok(())
}

/// RFC 3339 as chrono's `parse_from_rfc3339` reads it: `YYYY-MM-DD`, `T`/`t`/space, `HH:MM:SS`,
/// optional fraction, then `Z`/`z` or `±HH:MM`.
fn check_date(value: &str) -> Result<(), String> {
    let error = || format!("published_at {value:?} is not an RFC 3339 date");
    let b = value.as_bytes();
    let digits = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = b.get(range)?;
        part.iter()
            .all(u8::is_ascii_digit)
            .then(|| part.iter().fold(0u32, |n, d| n * 10 + u32::from(d - b'0')))
    };
    let fixed = (|| {
        let (month, day) = (digits(5..7)?, digits(8..10)?);
        let (hour, minute, second) = (digits(11..13)?, digits(14..16)?, digits(17..19)?);
        digits(0..4)?;
        let separators = b.get(4) == Some(&b'-')
            && b.get(7) == Some(&b'-')
            && matches!(b.get(10), Some(b'T' | b't' | b' '))
            && b.get(13) == Some(&b':')
            && b.get(16) == Some(&b':');
        (separators
            && (1..=12).contains(&month)
            && (1..=31).contains(&day)
            && hour < 24
            && minute < 60
            && second <= 60)
            .then_some(())
    })();
    fixed.ok_or_else(error)?;
    let mut rest = &b[19..];
    if rest.first() == Some(&b'.') {
        let fraction = rest[1..].iter().take_while(|d| d.is_ascii_digit()).count();
        if fraction == 0 {
            return Err(error());
        }
        rest = &rest[1 + fraction..];
    }
    let zone_ok = match rest {
        [b'Z' | b'z'] => true,
        [b'+' | b'-', h1, h2, b':', m1, m2] => {
            [h1, h2, m1, m2].iter().all(|d| d.is_ascii_digit())
                && (*h1 - b'0') * 10 + (*h2 - b'0') < 24
                && (*m1 - b'0') * 10 + (*m2 - b'0') < 60
        }
        _ => false,
    };
    zone_ok.then_some(()).ok_or_else(error)
}

fn schema_errors(schema: &str, value: &Value) -> Vec<String> {
    let schema_value: Value = match serde_json::from_str(schema) {
        Ok(schema_value) => schema_value,
        Err(error) => return vec![format!("vendored schema is not JSON: {error}")],
    };
    // The files are schemars 2020-12 output; nothing in them needs more than draft 7, which is
    // what this jsonschema version validates fully.
    let compiled = match JSONSchema::options()
        .with_draft(Draft::Draft7)
        .compile(&schema_value)
    {
        Ok(compiled) => compiled,
        Err(error) => return vec![format!("vendored schema does not compile: {error}")],
    };
    let errors = match compiled.validate(value) {
        Ok(()) => Vec::new(),
        Err(errors) => errors
            .map(|error| format!("{}: {error}", error.instance_path))
            .collect(),
    };
    errors
}

fn into_result(errors: Vec<String>) -> Result<(), Vec<String>> {
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn post() -> Value {
        json!({
            "type": "rzn.item/v2",
            "key": "x:post:1790000000000000001",
            "kind": "post",
            "url": "https://x.com/handle/status/1790000000000000001",
            "author": {"name": "Display Name", "id": "handle", "url": "https://x.com/handle"},
            "published_at": "2026-11-20T08:15:00.000Z",
            "published_in": {"name": "X", "id": "x.com", "url": "https://x.com"},
            "marker": "edit:0;posts:1",
            "fields": {"lang": "en", "is_article": false},
            "counts": {"likes": 4100, "views": 120000},
            "blocks": [
                {"id": "1790000000000000001", "kind": "post", "text": "First", "at": {"order": 0}},
                {"id": "q-1780000000000000009", "kind": "quoted", "text": "Quoted",
                 "at": {"order": 1}, "author": {"name": "Other", "id": "other"}}
            ]
        })
    }

    #[test]
    fn a_full_item_and_a_page_of_them_pass() {
        assert_eq!(check_item(&post()), Ok(()));
        let page = json!({"type": "rzn.items/v2", "items": [post()], "next_cursor": null, "listing": "feed"});
        assert_eq!(check_items_page(&page), Ok(()));
        assert_eq!(
            check_output(OutputContract::None, &json!("anything")),
            Ok(())
        );
    }

    #[test]
    fn full_items_need_author_date_place_and_a_body() {
        for field in ["author", "published_at", "published_in", "blocks"] {
            let mut item = post();
            item.as_object_mut().expect("object").remove(field);
            assert!(check_item(&item).is_err(), "{field} should be required");
            let page = json!({"type": "rzn.items/v2", "items": [item], "listing": "feed"});
            if field == "blocks" {
                assert_eq!(
                    check_items_page(&page),
                    Ok(()),
                    "a listing entry has no body"
                );
            } else {
                assert!(
                    check_items_page(&page).is_err(),
                    "{field} needed with a body"
                );
            }
        }
    }

    #[test]
    fn keys_listing_dates_counts_and_blocks_are_checked() {
        let page = |item: Value, listing: &str| {
            check_items_page(&json!({"type": "rzn.items/v2", "items": [item], "listing": listing}))
        };
        assert!(page(post(), "partial").is_err());
        let mut item = post();
        item["key"] = json!("1790000000000000001");
        assert!(page(item, "feed").is_err());
        let mut item = post();
        item["published_at"] = json!("Nov 20, 2026");
        assert!(page(item, "feed").is_err());
        let mut item = post();
        item["counts"]["likes"] = json!("4.1K");
        assert!(check_item(&item).is_err());
        let mut item = post();
        item["blocks"][1]["id"] = json!("1790000000000000001");
        assert!(check_item(&item).is_err());
        let mut item = post();
        item["blocks"][1]["parent"] = json!("later");
        assert!(check_item(&item).is_err());
        let mut item = post();
        item["truncated"] = json!({"reason": "thread_start_not_listed", "returned_blocks": 1});
        assert!(check_item(&item).is_err());
        assert_eq!(
            page(item, "feed"),
            Ok(()),
            "a truncated entry is only listed"
        );
    }

    #[test]
    fn dates_follow_rfc3339() {
        for good in [
            "2026-11-20T08:15:00Z",
            "2026-11-20t08:15:00.123z",
            "2026-11-20 08:15:00+05:30",
        ] {
            assert_eq!(check_date(good), Ok(()), "{good}");
        }
        for bad in [
            "2026-11-20",
            "2026-13-20T08:15:00Z",
            "2026-11-20T08:15:00",
            "2026-11-20T08:15:00.Z",
            "2026-11-20T08:15:00+0530",
        ] {
            assert!(check_date(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn describe_needs_kinds_with_key_shaped_identities() {
        let describe = json!({
            "type": "rzn.describe/v2",
            "connector": "x-browser",
            "kinds": [{"kind": "post", "identity": "x:post:<first post id>", "listing": "feed",
                "scopes": ["account"], "list_tool": "x/search_posts", "get_tool": "x/open",
                "fields": [{"name": "likes", "of": "item", "type": "number", "changes": true}]}]
        });
        assert_eq!(check_describe(&describe), Ok(()));
        let mut bad = describe.clone();
        bad["kinds"][0]["listing"] = json!("all");
        assert!(check_describe(&bad).is_err());
        let mut bad = describe;
        bad["kinds"] = json!([]);
        assert!(check_describe(&bad).is_err());
    }
}
