---
title: Workflows
slug: /workflows/
sidebar:
  label: Workflows
---

# Workflow Docs

This section is organized by system so each system can be routed cleanly in Astro and extended independently.

## Systems
- [Amazon](./amazon/README.md)
- [App Store](./appstore/README.md)
- [ASO](./aso/README.md)
- [G2](./g2/README.md)
- [Capterra](./capterra/README.md)
- [Etsy](./etsy/README.md)
- [ChatGPT](./chatgpt/README.md)
- [Claude](./claude/README.md)
- [Hacker News](./hn/README.md)
- [X](./x/README.md)

## Conventions
- One canonical workflow JSON filename per workflow.
- Versioning happens inside JSON (`id`, `version`), not via filename suffixes.
- Each system folder contains:
  - A system `README.md`
  - One markdown file per workflow
