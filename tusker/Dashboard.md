---
title: Vault Home
type: note
created: '2026-05-08'
updated: '2026-05-12'
tags:
- dashboard
- v6
---

# Vault Home

## Start Here

- [[SKILL]] routes agents through the V6 knowledge graph.
- [[README]] lists domains and epics.
- [[Docs]] lists knowledge nodes by domain.

## Domains

![[SKILL#Domains]]

## Active Work

Use `tusker validate`, `tusker knowledge route "<intent>"`, and `tusker knowledge list --domain <id>` for live terminal views. V6 source truth is `tusker/domains/**`; task proof is `tusker/epics/**`.

## Freshness

Run `tusker knowledge freshness --stale` after changing source anchors.
