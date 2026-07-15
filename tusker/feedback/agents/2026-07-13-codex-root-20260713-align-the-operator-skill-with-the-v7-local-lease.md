# Agent Feedback

- context: Interactive EUI-T-0002/EUI-T-0006 pickup in rzn-browser
- friction: Installed skill requires tusker runs claim, but that command rejected interactive pickup because the project was not registered for automation; tusker claim worked. Repo SKILL routes codebase/runtime files without the actual tusker/domains prefix.
- product-idea: Align the operator skill with the V7 local-lease command and make project-skill generated links resolve from the vault root.
- impact: Pickup required CLI help archaeology and repeated failed reads before implementation could begin.
- related: EUI-T-0002, EUI-T-0006
- dedupe-key: interactive-claim-and-domain-route-mismatch
