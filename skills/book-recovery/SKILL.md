---
name: book-recovery
description: Search and recover requested book batches with RZN Browser, test file mirrors, and verify editions and saved files before reporting completion.
---

# Book recovery

Use the [SOP](references/SOP.md) for batch recovery and failed-download diagnosis. The repository root is the directory containing this skill’s `skills/` parent; resolve installed symlinks before locating its `scripts/` and `workflows/`.

Establish the destination, requested titles/authors, edition constraints, and existing copies. Preserve existing files. A title match alone does not establish the requested author or edition.

Use reviewed exact IDs for retrieval. Choose mirrors per file using actual file responses and completed transfers. A responsive index page, a resolved URL, and a clicked Download button are intermediate states. A file is complete after its receipt, container, identity, and requested edition have been checked.

Keep one foreground batch controller per book browser tab. Resume by checking saved receipts. Expired IDs require a fresh search and review. Treat empty results, failed transfers, and human verification as distinct outcomes.

Use observed external links. Keep selected MD5 values, signed URLs, raw logs, and private selection records in local evidence. Adopt completed browser downloads only when their MD5 matches the reviewed exact copy. The ordinary book workflow supports LibGen tables and Anna cards; PW’s browser download is an operator fallback, not an automatic source adapter.

Stop at manual CAPTCHA and ask the user to complete it. After their reply, resume within the same scope. A purchase or subscription needs separate authorization; a sample, review scan, alternate author, or unsupported edition does not complete the request.

Finish with a verified file or an explicit skipped or unresolved status for every requested title, a report in the destination, edition differences, and useful access options for unresolved titles. Report measured mirror behavior as batch observations rather than a permanent ranking.
