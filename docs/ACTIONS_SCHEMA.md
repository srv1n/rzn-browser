# Actions Schema (Canonical)

## Canonical Source
- Canonical schema: `schema/actions-v1.json`
- Version: `schema_version` field inside the schema

## Consumers
- **Extension (TS)**: `extension/scripts/generate-types.ts` → `extension/src/types/actions.ts`
- **Rust**: `crates/rzn_core/build.rs` reads the schema for the action type list; `rzn_core::dsl::validate_action_value` validates steps.

## Drift Check
- Run: `make schema-check`
- Script: `scripts/check-actions-schema.js`

## Vendoring into rznapp
- Use: `scripts/vendor-actions-schema.sh /path/to/rznapp`
- Destination: `/path/to/rznapp/schema/actions-v1.json`

## Compatibility
- See `compatibility` section in `schema/actions-v1.json` for versioning and expectations.
